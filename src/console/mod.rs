//! The console.
//!
//! One page today: what this node is, and what state it is in. The
//! real console — projects, services, logs — arrives with the things
//! it would list.
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

pub mod assets;

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::Router;
// The renderer's own prelude — `rsx!` and every element name. It comes
// from this crate's direct `hypertext` dependency rather than through
// the framework, because the macro expands to absolute `::hypertext`
// paths; the framework re-export exists to check the versions agree.
use hypertext::prelude::*;
use hypertext::Raw;
use wabot::sqlite::SqliteDatabase;
use wabot::ui::hypertext::{link, style, title, IntoView};
use wabot::ui::{embedded_assets, UiResult, ViewBody};

use crate::config::Config;
use crate::edge::certs;

/// What the page reports. Read once per request — this is a status
/// page, and a cached status is the thing it exists not to be.
pub struct ConsoleState {
    database: Arc<SqliteDatabase>,
    config: Config,
}

impl ConsoleState {
    pub fn new(database: Arc<SqliteDatabase>, config: Config) -> Self {
        Self { database, config }
    }
}

/// How the node's TLS is currently answered for.
struct CertificateFacts {
    domain: Option<String>,
    /// `letsencrypt`, `letsencrypt-staging`, `self-signed`, …
    issuer: String,
    days_left: i64,
    trusted: bool,
}

async fn certificate_facts(state: &ConsoleState) -> CertificateFacts {
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

#[singleton]
pub struct HomeController {
    state: Arc<ConsoleState>,
}

#[ui_controller("/", app)]
impl HomeController {
    #[view("/")]
    async fn home(&self) -> UiResult<ViewBody> {
        let facts = certificate_facts(&self.state).await;
        let version = crate::api::VERSION;

        let hostname = facts
            .domain
            .clone()
            .unwrap_or_else(|| "not configured".into());
        let renews_in = facts.days_left - RENEW_WINDOW_DAYS;

        // The framework renders the document shell — doctype, html,
        // head, body — and a view supplies only what goes inside it.
        // Everything for the head is declared through the scope; a
        // view that emitted its own `<html>` would produce a second,
        // nested document, which is exactly what happened first.
        title("wabot-deploy");
        style(format!("{}/wabot.css", assets::MOUNT));
        link([
            ("rel", "icon"),
            ("type", "image/png"),
            ("href", &format!("{}/favicon.png", assets::MOUNT)),
        ]);

        Ok(rsx! {
            // XSS SAFETY: a `const` in this file, never a value from a
            // request.
            <style>(Raw::dangerously_create(PAGE_CSS))</style>
            <main class="shell">
                <header class="mark">
                    <img
                        src=(format!("{}/wabot-logo.png", assets::MOUNT))
                        alt="Wabot" width="44" height="44">
                    <div class="stack-sm">
                        <h1>("wabot-deploy")</h1>
                        <p class="tagline">("Container deployments on a node you own.")</p>
                    </div>
                </header>

                <section class="card">
                    <div class="split">
                        <p class="card-label">("This node")</p>
                        <span class="badge badge-success">
                            <span class="dot dot-success"></span>
                            ("Serving")
                        </span>
                    </div>
                    <dl class="kv">
                        <dt>("Version")</dt>
                        <dd>(version)</dd>
                        <dt>("Hostname")</dt>
                        <dd>(hostname)</dd>
                        <dt>("Certificate")</dt>
                        <dd>(&facts.issuer)</dd>
                        @if renews_in > 0 {
                            <dt>("Renews in")</dt>
                            <dd>(renews_in)(" days")</dd>
                        }
                    </dl>
                    <p class="note">(certificate_note(&facts))</p>
                </section>

                <section class="stack">
                    <p class="card-label">("What works so far")</p>
                    <div class="grid">
                        @for (name, detail) in CAPABILITIES {
                            <div class="card">
                                <p class="feature-name">(name)</p>
                                <p class="feature-detail">(detail)</p>
                            </div>
                        }
                    </div>
                </section>

                <section class="card card-sunken stack">
                    <p class="card-label">("Next")</p>
                    <p>(
                        "containerd and crun, so the node can start the containers \
                         it already knows how to route to."
                    )</p>
                    <pre><code>("wabot-deploy doctor")</code></pre>
                </section>

                <footer class="foot">
                    <span>("wabot-deploy ")(version)</span>
                    <span class="dim">(
                        "Single node. Two processes. No control plane but this one."
                    )</span>
                </footer>
            </main>
        }
        .into_view())
    }
}

/// Renewal starts this far before expiry, so "renews in" is the
/// remaining life minus the window rather than the whole of it.
const RENEW_WINDOW_DAYS: i64 = 30;

/// What the node can do, in the order somebody would meet it.
const CAPABILITIES: &[(&str, &str)] = &[
    ("Install", "Converges. Run it again and nothing repeats."),
    (
        "Edge",
        "One TLS listener. Host routing, WebSockets, HTTP redirect.",
    ),
    (
        "Certificates",
        "Let's Encrypt over HTTP-01, renewed in the background.",
    ),
    (
        "Storage",
        "SQLite, compiled in. No database server to operate.",
    ),
];

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

/// Page-specific layout only.
///
/// Everything visual — colour, type, radii — comes from the design
/// system's tokens. What is here is arrangement, which is the page's
/// own business. No borders, no shadows, no hover states: separation
/// is background contrast, per the brand rules.
const PAGE_CSS: &str = r#"
.shell {
  max-width: 62rem;
  margin: 0 auto;
  padding: var(--sp-16) var(--sp-6) var(--sp-12);
  display: flex;
  flex-direction: column;
  gap: var(--sp-10);
}
.mark { display: flex; align-items: center; gap: var(--sp-5); }
.mark h1 { font-size: var(--fs-4xl); margin: 0; letter-spacing: -0.03em; }
.tagline { color: rgb(var(--c-fg-muted)); font-size: var(--fs-lg); margin: 0; }
.status .kv { margin-top: var(--sp-5); }
.note {
  margin: var(--sp-5) 0 0;
  color: rgb(var(--c-fg-muted));
  font-size: var(--fs-sm);
  max-width: 46rem;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(15rem, 1fr));
  gap: var(--sp-4);
}
.feature-name { margin: 0 0 var(--sp-2); font-weight: 600; }
.feature-detail { margin: 0; color: rgb(var(--c-fg-muted)); font-size: var(--fs-sm); }
.foot {
  display: flex;
  justify-content: space-between;
  gap: var(--sp-4);
  flex-wrap: wrap;
  font-size: var(--fs-sm);
  color: rgb(var(--c-fg-muted));
}
.foot .dim { color: rgb(var(--c-fg-faint)); }
@media (max-width: 40rem) {
  .shell { padding-top: var(--sp-10); gap: var(--sp-8); }
  .mark h1 { font-size: var(--fs-3xl); }
}
"#;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

pub fn register(container: &Container, database: Arc<SqliteDatabase>, config: Config) {
    container.register_instance::<ConsoleState>(Arc::new(ConsoleState::new(database, config)));
    register_singletons!(container, HomeController);
}

pub fn routes(container: &Container) -> Router {
    HomeController::register_ui_routes(container, wabot::ui::ui_router())
        .merge(embedded_assets(assets::MOUNT, assets::ASSETS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wabot::rest::axum::http::StatusCode;
    use wabot::testing::RestHarness;

    async fn harness(domain: Option<&str>) -> RestHarness {
        let database = Arc::new(crate::db::open_in_memory().await.expect("open"));
        let mut config = Config::default();
        config.node.domain = domain.map(str::to_string);

        let container = Container::new();
        register(&container, database, config);
        RestHarness::new(routes(&container))
    }

    #[tokio::test]
    async fn the_home_page_renders() {
        let response = harness(None).await.get("/").send().await;
        response.assert_ok();
        assert!(response.body.contains("wabot-deploy"), "{}", response.body);
        assert!(
            response
                .body
                .contains("Container deployments on a node you own"),
            "the tagline is there"
        );
    }

    /// The page's job is to say what state the node is in, so the
    /// certificate line has to reflect reality rather than a default.
    #[tokio::test]
    async fn the_page_reports_a_self_signed_node_honestly() {
        let response = harness(None).await.get("/").send().await;
        assert!(
            response.body.contains("self-signed"),
            "a node with no domain says so: {}",
            response.body
        );
        assert!(response.body.contains("node.domain"), "and says what to do");
    }

    /// One document, not two.
    ///
    /// The framework renders the shell and the view supplies its body.
    /// A view that emits its own `<html>` produces a nested document —
    /// which browsers silently repair, so it renders *almost* right
    /// and nothing complains. That is what this file did first.
    #[tokio::test]
    async fn the_page_is_a_single_document() {
        let body = harness(None).await.get("/").send().await.body;

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
        let body = harness(None).await.get("/").send().await.body;

        assert!(body.contains("<title>wabot-deploy</title>"), "{body}");
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

    /// Every asset the page references has to be served, or the node
    /// renders unstyled — which is exactly the failure vendoring them
    /// was supposed to prevent.
    #[tokio::test]
    async fn every_referenced_asset_is_served() {
        let harness = harness(None).await;
        let body = harness.get("/").send().await.body;

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
                harness.get(url).send().await.assert_status(StatusCode::OK);
                checked += 1;
            }
        }
        assert!(checked >= 3, "the page references its assets: {checked}");
    }

    /// A stylesheet the browser cannot parse renders an unstyled page,
    /// which looks like a bug in the product rather than in a header.
    #[tokio::test]
    async fn the_stylesheet_is_served_as_css() {
        let response = harness(None)
            .await
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
}
