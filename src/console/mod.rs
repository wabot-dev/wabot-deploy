//! The console.
//!
//! Server-rendered with hypertext's `rsx!`. No JavaScript, no build
//! step, no client payload: the page is HTML the node produced, which
//! is the right shape for something that has to work on a box with no
//! internet access and under a gigabyte of RAM.
//!
//! ## Why hypertext and not Maud
//!
//! Both compile markup to string-pushing at build time and escape
//! interpolated values by construction. hypertext also **validates
//! element and attribute names**, so `<dvi klass="x">` is a build
//! error. Maud compiles it and emits the malformed HTML — checked,
//! not assumed. For templates nobody type-checks by reading them,
//! that difference is the whole point.
//!
//! ## Who may see what
//!
//! Every page except `/setup` and `/sign-in` needs an account. The
//! check is per-view rather than per-router because a signed-out
//! visitor gets a *redirect*, and a router-level guard can only
//! reject. See [`auth`].

pub mod assets;
pub mod auth;
pub mod layout;
pub mod projects;
pub mod services;

use std::sync::Arc;

use hypertext::prelude::*;
use wabot::prelude::*;
use wabot::rest::axum::Router;
use wabot::sqlite::SqliteDatabase;
use wabot::ui::embedded_assets;

use crate::config::Config;
use crate::edge::certs;

/// What every console page reads from. Never cached: this is a status
/// console, and a cached status is the thing it exists not to be.
pub struct ConsoleState {
    pub(crate) database: Arc<SqliteDatabase>,
    pub(crate) config: Config,
    pub(crate) deployer: Arc<crate::deploy::Deployer>,
}

impl ConsoleState {
    pub fn new(database: Arc<SqliteDatabase>, config: Config) -> Self {
        let deployer = Arc::new(crate::deploy::Deployer::new(
            database.clone(),
            &config.node.data_dir,
        ));
        Self {
            database,
            config,
            deployer,
        }
    }
}

/// How the node's TLS is currently answered for.
pub(crate) struct CertificateFacts {
    pub domain: Option<String>,
    /// `letsencrypt`, `letsencrypt-staging`, `self-signed`, …
    pub issuer: String,
    pub days_left: i64,
    pub trusted: bool,
}

pub(crate) async fn certificate_facts(state: &ConsoleState) -> CertificateFacts {
    let domain = state.config.node.domain.clone();
    let stored = match &domain {
        Some(domain) => certs::load(&state.database, domain).await.ok().flatten(),
        None => None,
    };

    match stored {
        Some(certificate) => {
            let issuer = short_issuer(&certificate.issuer);
            CertificateFacts {
                days_left: (certificate.not_after - now_ms()) / 86_400_000,
                // Staging roots are untrusted by design, and saying so
                // here saves somebody working out why their browser
                // still complains.
                trusted: issuer == "letsencrypt",
                issuer,
                domain,
            }
        }
        None => CertificateFacts {
            domain,
            issuer: "self-signed".into(),
            days_left: 0,
            trusted: false,
        },
    }
}

fn short_issuer(issuer: &str) -> String {
    match issuer {
        "self-signed" => "self-signed".into(),
        url if url.contains("acme-staging-v02.api.letsencrypt.org") => "letsencrypt-staging".into(),
        url if url.contains("acme-v02.api.letsencrypt.org") => "letsencrypt".into(),
        url => url.split('/').nth(2).unwrap_or(url).into(),
    }
}

/// Renewal starts this far before expiry, so "renews in" is the
/// remaining life minus the window rather than the whole of it.
const RENEW_WINDOW_DAYS: i64 = 30;

/// What the node is, as a card at the foot of the projects page.
///
/// It stays on the page somebody lands on rather than getting its own
/// route: the questions it answers — which certificate, how long left
/// — are the ones asked while looking at something else.
pub(crate) fn node_card(facts: &CertificateFacts) -> impl Renderable + '_ {
    let hostname = facts.domain.clone().unwrap_or_else(|| "not set".into());
    let renews_in = facts.days_left - RENEW_WINDOW_DAYS;

    rsx! {
        <section class="card card-sunken">
            <div class="split">
                <p class="card-label">("This node")</p>
                <span class="badge badge-success">
                    <span class="dot dot-success"></span>
                    ("Serving")
                </span>
            </div>
            <dl class="kv">
                <dt>("Version")</dt>
                <dd>(crate::api::VERSION)</dd>
                <dt>("Hostname")</dt>
                <dd>(hostname)</dd>
                <dt>("Certificate")</dt>
                <dd>(&facts.issuer)</dd>
                @if renews_in > 0 {
                    <dt>("Renews in")</dt>
                    <dd>(renews_in)(" days")</dd>
                }
            </dl>
            <p class="note">(certificate_note(facts))</p>
        </section>
    }
}

/// What to say about the certificate currently being served.
///
/// Plain text rather than markup: the three cases differ only in
/// wording, and a function returning a string is easier to read than
/// three branches of near-identical HTML.
fn certificate_note(facts: &CertificateFacts) -> &'static str {
    if facts.trusted {
        "This page reached you over TLS with a publicly trusted certificate \
         the node obtained and installed on its own."
    } else if facts.issuer == "letsencrypt-staging" {
        "Staging certificate — browsers will not trust it, which is expected. \
         Set acme.directory to production when you are done testing."
    } else {
        "Self-signed, from this node's local authority. Set node.domain and \
         the node will obtain a public one."
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

pub fn register(container: &Container, database: Arc<SqliteDatabase>, config: Config) {
    container.register_instance::<ConsoleState>(Arc::new(ConsoleState::new(database, config)));
    // No guard addon runs here, so nothing else would register `Auth`
    // and every controller holding one would fail to resolve.
    Auth::register_default(container);

    register_singletons!(container, auth::SessionMiddleware);
    // Transient, all of them: each holds an `Arc<Auth>`, and a
    // singleton controller is built once — its `Auth` would be one
    // visitor's identity handed to everybody after them.
    register_transients!(
        container,
        auth::AuthPages,
        auth::AuthApi,
        projects::ProjectPages,
        projects::ProjectApi,
        services::ServicePages,
        services::ServiceApi
    );
}

pub fn routes(container: &Container) -> Router {
    let pages = wabot::ui::ui_router();
    let pages = auth::AuthPages::register_ui_routes(container, pages);
    let pages = projects::ProjectPages::register_ui_routes(container, pages);
    let pages = services::ServicePages::register_ui_routes(container, pages);

    let forms = Router::new();
    let forms = auth::AuthApi::register_routes(container, forms);
    let forms = projects::ProjectApi::register_routes(container, forms);
    let forms = services::ServiceApi::register_routes(container, forms);

    pages
        .merge(forms)
        .merge(embedded_assets(assets::MOUNT, assets::ASSETS))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use wabot::rest::axum::http::StatusCode;
    use wabot::testing::RestHarness;

    /// A console over an empty database, with a setup token already
    /// issued — what `install` leaves behind.
    pub(crate) struct Console {
        pub harness: RestHarness,
        pub database: Arc<SqliteDatabase>,
        pub setup_token: String,
    }

    impl Console {
        pub async fn new() -> Self {
            let database = Arc::new(crate::db::open_in_memory().await.expect("open"));
            let setup_token = crate::accounts::issue_setup_token(&database)
                .await
                .expect("token");

            let container = Container::new();
            register(&container, database.clone(), Config::default());
            Self {
                harness: RestHarness::new(routes(&container)),
                database,
                setup_token,
            }
        }

        /// Complete setup and return the session cookie, in the form a
        /// request carries it.
        pub async fn signed_in(&self) -> String {
            let response = self
                .harness
                .post("/setup")
                .form(&[
                    ("username", "jorge"),
                    ("password", "correct horse battery"),
                    ("setup_token", &self.setup_token),
                ])
                .send()
                .await;
            response.assert_status(StatusCode::SEE_OTHER);
            let cookie = response
                .header("set-cookie")
                .expect("setup signs the operator in");
            cookie
                .split(';')
                .next()
                .expect("a cookie has a value")
                .to_string()
        }
    }

    /// The console the framework assembles has to be one document, not
    /// a document inside a document — which browsers silently repair,
    /// so it renders *almost* right and nothing complains.
    #[tokio::test]
    async fn a_page_is_a_single_document() {
        let console = Console::new().await;
        let body = console.harness.get("/setup").send().await.body;

        assert_eq!(body.matches("<html").count(), 1, "one <html>:\n{body}");
        assert_eq!(body.matches("<body").count(), 1, "one <body>");
        assert_eq!(body.matches("<!doctype").count(), 1, "one doctype");
        assert_eq!(
            body.to_lowercase().matches("<title").count(),
            1,
            "one <title>"
        );
    }

    /// The head is assembled from what the view declared through the
    /// scope, so a missing helper shows up as a missing tag.
    #[tokio::test]
    async fn the_head_carries_the_title_stylesheet_and_favicon() {
        let console = Console::new().await;
        let body = console.harness.get("/setup").send().await.body;

        assert!(body.contains("<title>"), "{body}");
        assert!(
            body.contains(&format!(
                r#"<link rel="stylesheet" href="{}/wabot.css">"#,
                assets::MOUNT
            )),
            "the stylesheet is linked"
        );
        assert!(
            body.contains(r#"rel="icon""#),
            "the favicon is linked — the reason `add_link` exists"
        );
    }

    /// The whole point of the preloads: they have to come *before* the
    /// stylesheet, or the browser still discovers the fonts late and
    /// the flicker stays.
    ///
    /// The framework's document assembly is what guarantees this —
    /// links are emitted before stylesheets regardless of declaration
    /// order — so this test watches an invariant the view does not
    /// control. Worth having for exactly that reason: if that ordering
    /// ever changes upstream, the flicker comes back silently and this
    /// is what says so.
    #[tokio::test]
    async fn fonts_are_preloaded_before_the_stylesheet() {
        let console = Console::new().await;
        let body = console.harness.get("/setup").send().await.body;

        let stylesheet = body
            .find(r#"rel="stylesheet""#)
            .expect("the stylesheet is linked");

        for font in assets::PRELOAD_FONTS {
            let preload = body
                .find(&format!(r#"href="{}/{font}""#, assets::MOUNT))
                .unwrap_or_else(|| panic!("{font} is not preloaded:\n{body}"));
            assert!(
                preload < stylesheet,
                "{font} is preloaded after the stylesheet, which defeats it"
            );
        }
    }

    /// A font preload without `crossorigin` is discarded and the font
    /// fetched again — a wasted request that fixes nothing. The one
    /// mistake worth a test of its own.
    #[tokio::test]
    async fn font_preloads_carry_the_attributes_the_browser_needs() {
        let console = Console::new().await;
        let body = console.harness.get("/setup").send().await.body;

        let preloads: Vec<&str> = body
            .match_indices(r#"<link rel="preload""#)
            .map(|(start, _)| {
                let rest = &body[start..];
                &rest[..rest.find('>').map(|end| end + 1).unwrap_or(rest.len())]
            })
            .collect();

        assert_eq!(
            preloads.len(),
            assets::PRELOAD_FONTS.len(),
            "one preload per declared font: {preloads:?}"
        );
        for preload in preloads {
            for attribute in [
                r#"as="font""#,
                r#"type="font/woff2""#,
                r#"crossorigin="anonymous""#,
            ] {
                assert!(
                    preload.contains(attribute),
                    "a font preload needs {attribute}: {preload}"
                );
            }
        }
    }

    /// Every asset a page references has to be served, or the node
    /// renders unstyled — which is exactly the failure vendoring them
    /// was supposed to prevent.
    #[tokio::test]
    async fn every_referenced_asset_is_served() {
        let console = Console::new().await;
        let body = console.harness.get("/setup").send().await.body;

        let mut checked = 0;
        for marker in ["href=\"", "src=\""] {
            let mut rest = body.as_str();
            while let Some(start) = rest.find(marker) {
                rest = &rest[start + marker.len()..];
                let Some(end) = rest.find('"') else { break };
                let url = &rest[..end];
                if !url.starts_with(assets::MOUNT) {
                    continue;
                }
                console
                    .harness
                    .get(url)
                    .send()
                    .await
                    .assert_status(StatusCode::OK);
                checked += 1;
            }
        }
        assert!(checked >= 3, "the page references its assets: {checked}");
    }

    /// A stylesheet the browser cannot parse renders an unstyled page,
    /// which looks like a bug in the product rather than in a header.
    #[tokio::test]
    async fn the_stylesheet_is_served_as_css() {
        let console = Console::new().await;
        let response = console
            .harness
            .get(&format!("{}/wabot.css", assets::MOUNT))
            .send()
            .await;
        response.assert_ok();
        assert!(
            response
                .header("content-type")
                .is_some_and(|value| value.starts_with("text/css")),
            "got {:?}",
            response.header("content-type")
        );
    }

    /// The node card is what the splash page became, and the honesty
    /// of the certificate line is the reason it exists.
    #[tokio::test]
    async fn the_node_card_reports_a_self_signed_node_honestly() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let body = console
            .harness
            .get("/")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(body.contains("self-signed"), "{body}");
        assert!(body.contains("node.domain"), "and says what to do");
    }
}
