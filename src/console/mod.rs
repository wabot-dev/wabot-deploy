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
pub mod nodes;
pub mod people;
pub mod projects;
pub mod services;
pub mod shell;
pub mod updates;

use std::sync::Arc;

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
    /// What releases exist, as last read from GitHub. Shared so that
    /// the page and the update it starts agree about what they are
    /// installing.
    pub(crate) catalogue: Arc<crate::update::Catalogue>,
    /// Secrets waiting for the one page that will show them.
    pub(crate) reveals: Reveals,
    /// The certificate loop, to ask it to look now and to watch what
    /// it is doing. Held here as well as inside the deployer because
    /// the node page reports on it, which is not deploying.
    pub(crate) certificates: Arc<crate::edge::acme::Wake>,
    /// For enqueuing work. `run_command` is a free function over the
    /// container — the framework's reasoning is that an `Async`
    /// singleton would have to hold a container that holds it — so the
    /// container is what a caller needs. It is a cheap clonable handle.
    pub(crate) container: Container,
}

/// A secret handed from a POST to the GET that follows it.
///
/// A push token exists in clear for exactly the moment it is minted,
/// and the page that shows it is reached by redirect. It used to travel
/// there in the query string — which put it in the address bar, in the
/// browser's history, and back on the screen on every refresh. This
/// console's own rule is that a query parameter is fine *because it is
/// not secret* (see `back_with_error`), and that one was the exception
/// nobody noticed.
///
/// So the redirect now carries a nonce that names the secret, and the
/// nonce is spent when the page reads it. In memory rather than in the
/// database because the plaintext must not outlive the process, and a
/// restart losing an unread secret is the right failure — the token is
/// revocable and another one costs a click.
#[derive(Default)]
pub(crate) struct Reveals {
    held: std::sync::Mutex<std::collections::HashMap<String, (String, i64)>>,
}

/// How long a minted secret waits to be read. Long enough for a
/// redirect, short enough that a browser left on the login screen does
/// not keep one in memory all afternoon.
const REVEAL_TTL_MS: i64 = 60_000;

impl Reveals {
    /// Hold a secret, returning the nonce that names it.
    pub(crate) fn stash(&self, secret: String) -> String {
        let nonce = wabot::prelude::password::generate(24);
        let now = now_ms();
        let mut held = self.held.lock().expect("no panic holds this lock");
        // Swept here rather than on a timer: the only way the map
        // grows is somebody minting a token, so the act of growing it
        // is the right moment to drop what expired.
        held.retain(|_, (_, at)| now - *at < REVEAL_TTL_MS);
        held.insert(nonce.clone(), (secret, now));
        nonce
    }

    /// Read a secret, once.
    pub(crate) fn take(&self, nonce: &str) -> Option<String> {
        let now = now_ms();
        let mut held = self.held.lock().expect("no panic holds this lock");
        held.remove(nonce)
            .filter(|(_, at)| now - *at < REVEAL_TTL_MS)
            .map(|(secret, _)| secret)
    }
}

impl ConsoleState {
    /// Where this console answers, for a link somebody has to paste
    /// somewhere else. Built from the node's own domain, because a
    /// link to `localhost` is one only the node can open.
    pub(crate) async fn base_url(&self) -> String {
        let host = crate::node::settings::domain(&self.database, &self.config)
            .await
            .unwrap_or_else(|| "localhost".into());
        match self.config.edge.https_port {
            443 => format!("https://{host}"),
            port => format!("https://{host}:{port}"),
        }
    }

    pub fn new(
        container: Container,
        database: Arc<SqliteDatabase>,
        config: Config,
        routes: Arc<crate::edge::routes::RouteTable>,
        certificates: Arc<crate::edge::acme::Wake>,
    ) -> Self {
        let deployer = Arc::new(
            crate::deploy::Deployer::new(database.clone(), &config)
                .with_routes(routes)
                .with_certificates(certificates.clone()),
        );
        Self {
            database,
            config,
            deployer,
            catalogue: Arc::new(crate::update::Catalogue::default()),
            reveals: Reveals::default(),
            certificates,
            container,
        }
    }
}

/// How the node's TLS is currently answered for.
pub(crate) struct CertificateFacts {
    /// `letsencrypt`, `letsencrypt-staging`, `self-signed`, or whatever
    /// common name signed a certificate found on disk. A fact.
    pub issuer: String,
    pub days_left: i64,
    pub trusted: bool,
    /// How the node came by it, which is a different question — see
    /// migration `0012`.
    pub source: crate::edge::certs::Source,
}

impl CertificateFacts {
    /// How long until renewal starts, or `None` when nothing will
    /// renew — a self-signed certificate reports zero days left, and
    /// subtracting the window from that is a negative number about an
    /// event that is not coming.
    /// What a name with nothing issued for it looks like: the node is
    /// serving it off the local authority's certificate, which is what
    /// `refresh_local` keeps covering every name.
    pub(crate) fn none() -> Self {
        Self {
            issuer: "self-signed".into(),
            days_left: 0,
            trusted: false,
            source: crate::edge::certs::Source::SelfSigned,
        }
    }

    pub(crate) fn renews_in_days(&self) -> Option<i64> {
        let renews_in = self.days_left - RENEW_WINDOW_DAYS;
        (renews_in > 0).then_some(renews_in)
    }
}

pub(crate) async fn certificate_facts(state: &ConsoleState) -> CertificateFacts {
    // The domain is not carried back out: every caller already knows it
    // — they render the form that sets it — and two copies of a value
    // one page shows twice is how they end up disagreeing.
    match crate::node::settings::domain(&state.database, &state.config).await {
        Some(domain) => certificate_facts_for(state, &domain).await,
        None => CertificateFacts::none(),
    }
}

/// The same, for any name the node answers for.
///
/// A service hostname has a certificate of its own, with its own
/// source and its own expiry — the node's domain is not a special case
/// of anything, it is just the name that had a page first.
pub(crate) async fn certificate_facts_for(state: &ConsoleState, name: &str) -> CertificateFacts {
    let stored = certs::load(&state.database, name).await.ok().flatten();

    match stored {
        Some(certificate) => {
            let issuer = short_issuer(&certificate.issuer);
            CertificateFacts {
                days_left: (certificate.not_after - now_ms()) / 86_400_000,
                // Staging roots are untrusted by design, and saying so
                // here saves somebody working out why their browser
                // still complains.
                //
                // A certificate found on disk is not claimed either
                // way: whether a browser trusts it depends on a trust
                // store this node cannot see, and guessing would be the
                // console asserting something it does not know.
                trusted: issuer == "letsencrypt",
                issuer,
                source: certificate.source,
            }
        }
        None => CertificateFacts::none(),
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

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

pub fn register(
    container: &Container,
    database: Arc<SqliteDatabase>,
    config: Config,
    routes: Arc<crate::edge::routes::RouteTable>,
    certificates: Arc<crate::edge::acme::Wake>,
) {
    container.register_instance::<ConsoleState>(Arc::new(ConsoleState::new(
        container.clone(),
        database,
        config,
        routes,
        certificates,
    )));
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
        nodes::NodePages,
        nodes::NodeApi,
        people::PeoplePages,
        people::PeopleApi,
        projects::ProjectPages,
        projects::ProjectApi,
        services::ServicePages,
        services::ServiceApi,
        updates::UpdatePages,
        updates::UpdateApi
    );
}

pub fn routes(container: &Container) -> Router {
    let pages = wabot::ui::ui_router();
    let pages = auth::AuthPages::register_ui_routes(container, pages);
    let pages = nodes::NodePages::register_ui_routes(container, pages);
    let pages = people::PeoplePages::register_ui_routes(container, pages);
    let pages = projects::ProjectPages::register_ui_routes(container, pages);
    let pages = services::ServicePages::register_ui_routes(container, pages);
    let pages = updates::UpdatePages::register_ui_routes(container, pages);

    let forms = Router::new();
    let forms = auth::AuthApi::register_routes(container, forms);
    let forms = nodes::NodeApi::register_routes(container, forms);
    let forms = people::PeopleApi::register_routes(container, forms);
    let forms = projects::ProjectApi::register_routes(container, forms);
    let forms = services::ServiceApi::register_routes(container, forms);
    let forms = updates::UpdateApi::register_routes(container, forms);

    pages
        .merge(forms)
        .merge(embedded_assets(assets::MOUNT, assets::ASSETS))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use wabot::async_jobs::{
        register_async_runtime, register_cron_job_repository, register_job_repository,
    };
    use wabot::rest::axum::http::StatusCode;
    use wabot::testing::RestHarness;

    /// A console over an empty database, with a setup token already
    /// issued — what `install` leaves behind.
    pub(crate) struct Console {
        pub harness: RestHarness,
        /// The same router, asked the way the client runtime asks it.
        /// `navigate` fetches a boosted-navigation fragment, which is
        /// how every in-console link actually arrives.
        pub ui: wabot::testing::UiHarness,
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
            register(
                &container,
                database.clone(),
                Config::default(),
                Arc::new(crate::edge::routes::RouteTable::new()),
                Arc::new(crate::edge::acme::Wake::default()),
            );
            // The queue, so a test exercises the path a node takes:
            // a POST enqueues, `run_command` spawns, the handler runs.
            // Without it `run_command` fails to resolve a repository
            // and every deployment quietly does not happen.
            register_job_repository(
                &container,
                Arc::new(wabot_addon_async_in_memory::InMemoryJobRepository::new()),
            );
            register_cron_job_repository(
                &container,
                Arc::new(wabot_addon_async_in_memory::InMemoryCronJobRepository::new()),
            );
            register_async_runtime(&container);
            // The handler wants both, and `console::register` registers
            // neither — on a node `api::register` puts the database in.
            container.register_instance::<SqliteDatabase>(database.clone());
            container.register_instance::<crate::deploy::Deployer>(Arc::new(
                crate::deploy::Deployer::new(database.clone(), &Config::default()),
            ));
            register_transients!(&container, crate::deploy::jobs::DeployHandler);
            // What `run_async_workers` does with its entries, minus the
            // workers: the executor only runs a command it knows, and
            // `run_command`'s immediate spawn checks exactly that.
            //
            // And the executor keeps its *own* set of names, which is
            // what `run_command`'s immediate spawn checks — registering
            // with the `CommandRegistry` alone leaves the job stored
            // and never run.
            let commands: Arc<wabot::async_jobs::CommandRegistry> = container.resolve();
            commands.register(crate::deploy::jobs::DeployHandler::__handler_entry(
                &container,
            ));
            let executor: Arc<wabot::async_jobs::JobExecutor> = container.resolve();
            executor.add_command(crate::deploy::jobs::DeployService::COMMAND_NAME);

            let router = routes(&container);
            Self {
                harness: RestHarness::new(router.clone()),
                ui: wabot::testing::UiHarness::new(router),
                database,
                setup_token,
            }
        }

        /// Invite somebody, accept it, and return their session cookie.
        ///
        /// The console's own path from "an administrator" to "a second
        /// person with less power", which is what most of these tests
        /// need before they can check that the second person is
        /// refused something.
        pub async fn joined_as(&self, admin_cookie: &str, username: &str) -> String {
            let response = self
                .harness
                .post("/people/invite")
                .header("cookie", admin_cookie)
                .form(&[("node_role", "member")])
                .send()
                .await;
            let location = response.header("location").expect("redirected");
            let query = location.split_once('?').expect("carries the link").1;
            let link = form_urlencoded::parse(query.as_bytes())
                .find(|(key, _)| key == "invited")
                .map(|(_, value)| value.into_owned())
                .expect("the link");
            let token = link.rsplit('/').next().expect("the token").to_string();

            let joined = self
                .harness
                .post(&format!("/join/{token}"))
                .form(&[
                    ("username", username),
                    ("password", "a long passphrase here"),
                ])
                .send()
                .await;
            joined
                .header("set-cookie")
                .expect("signed in")
                .split(';')
                .next()
                .expect("a cookie has a value")
                .to_string()
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

    /// The theme is in the first byte the browser gets, which is the
    /// whole reason it lives on the account rather than in a script.
    /// A theme applied after paint is a flash of the one somebody just
    /// turned off.
    #[tokio::test]
    async fn the_chosen_theme_is_server_rendered() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let before = console
            .harness
            .get("/")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            before.contains(r#"data-theme="""#),
            "system means no attribute, so the media query decides: {before}"
        );

        console
            .harness
            .post("/theme")
            .header("cookie", &cookie)
            .form(&[("theme", "dark"), ("from", "/")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let after = console
            .harness
            .get("/")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(after.contains(r#"data-theme="dark""#), "{after}");
        assert!(
            !after.contains("<script>document.documentElement"),
            "and nothing had to run to get it there"
        );
    }

    /// `from` is a path this console put in its own form. Taking it
    /// from the Referer would let a page elsewhere choose where a
    /// submit lands.
    #[tokio::test]
    async fn the_theme_form_only_returns_to_a_path() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .post("/theme")
            .header("cookie", &cookie)
            .form(&[("theme", "dark"), ("from", "https://elsewhere.example/")])
            .send()
            .await;
        assert_eq!(response.header("location"), Some("/"));
    }

    /// Both islands are declared where the runtime will look for them.
    /// A host that stops being emitted takes its behaviour with it and
    /// nothing errors — the page just quietly stops doing the thing.
    #[tokio::test]
    async fn the_console_declares_the_islands_it_registers() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let ui = console.ui.with_header("cookie", cookie);

        let node = ui.get("/nodes/local").await;
        assert!(node.has_island("node-live"), "{}", node.html());
        assert_eq!(
            node.island_props("node-live"),
            Some(serde_json::json!({ "node": "local" })),
            "the client needs the id to open the stream"
        );
        assert!(node.has_island("fields"), "and the certificate form");
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

    /// A node serving its own authority's certificate has to say so.
    /// The sentence outlived two homes — a splash page, then a summary
    /// card — and what makes it worth keeping is that it names what to
    /// do next, beside the form that does it.
    #[tokio::test]
    async fn a_self_signed_node_says_so_where_it_can_be_fixed() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let body = console
            .harness
            .get(&format!("/nodes/{}", crate::node::LOCAL_ID))
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(body.contains("self-signed"), "{body}");
        assert!(body.contains("below"), "and points at the form: {body}");
        assert!(
            body.contains("/nodes/certificate"),
            "which is on this page: {body}"
        );
    }

    /// The list said the node's name, version and state; the card under
    /// it said the same three things again. Two cards agreeing is not
    /// confirmation, it is a page that has not decided what it is for.
    #[tokio::test]
    async fn the_nodes_list_says_each_thing_once() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let body = console
            .harness
            .get("/nodes")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert_eq!(
            body.matches("Serving").count(),
            1,
            "one node, said once: {body}"
        );
        assert!(
            !body.contains("Certificate"),
            "the certificate lives on the node's own page: {body}"
        );
    }
}
