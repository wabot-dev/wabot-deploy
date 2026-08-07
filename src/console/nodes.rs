//! Nodes: the list, and what one is spending.
//!
//! ## The only JavaScript in the product
//!
//! Memory changes every second, and a page that shows a number from
//! when it was rendered is a page somebody reloads to check. So this
//! one view carries a script: an `EventSource` that replaces the
//! figures in place.
//!
//! It is deliberately the *whole* of the client's involvement. The
//! page renders complete and correct with scripting off — the script
//! only replaces text that is already there — so the console keeps
//! working on a machine where the stream cannot be opened, which is
//! the machine somebody is most likely looking at this page from.

use std::collections::BTreeMap;
use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use crate::node::memory::{self, Snapshot};
use crate::platform::access;

use super::auth::{signed_in, SessionMiddleware};
use super::shell::{Area, Frame};
use super::{layout, ConsoleState};

#[derive(Debug, Deserialize, Validate)]
pub struct NodePage {
    pub node: String,
    pub error: Option<String>,
    /// What a check or a request just said, carried back from the POST
    /// that ran it.
    pub checked: Option<String>,
}

#[injectable]
pub struct NodePages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl NodePages {
    #[view("/nodes")]
    #[middleware(SessionMiddleware)]
    async fn index(&self) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        // The node belongs to whoever runs it. Hiding the link is
        // courtesy; this is the boundary.
        if !account.is_admin() {
            return Ok(Redirect::found("/").into());
        }

        let nodes = crate::node::all(
            crate::node::settings::domain(&self.state.database, &self.state.config).await,
        );
        let projects = access::projects_for(&self.state.database, &account).await?;
        let facts = super::certificate_facts(&self.state).await;

        layout::head("Nodes");
        let frame = Frame::new(&account, Area::Nodes, &projects, None, "/nodes");
        let body = rsx! {
                (layout::style_tag())
                <h1>("Nodes")</h1>
                <p class="tagline">(
                    "One node runs this console and everything on it. \
                     Joining a second is not a thing yet."
                )</p>

                <div class="grid">
                    @for node in &nodes {
                        <a class="card tile" href=(format!("/nodes/{}", node.id))>
                            <div class="split">
                                <p class="tile-name">(&node.name)</p>
                                <span class="badge badge-success">
                                    <span class="dot dot-success"></span>("Serving")
                                </span>
                            </div>
                            <p class="tile-detail">
                                ("wabot-deploy ")(node.version)
                                @if node.is_self { (" · this node") }
                            </p>
                        </a>
                    }
                </div>

                (super::node_card(&facts))
        }
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }

    #[view("/nodes/:node")]
    #[middleware(SessionMiddleware)]
    async fn node(&self, query: NodePage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !account.is_admin() {
            return Ok(Redirect::found("/").into());
        }
        let domain = crate::node::settings::domain(&self.state.database, &self.state.config).await;
        let Some(node) = crate::node::find(domain.clone(), &query.node) else {
            return Ok(Redirect::found("/nodes").into());
        };

        let projects = access::projects_for(&self.state.database, &account).await?;
        let snapshot = self.state.deployer.memory().await;
        let facts = super::certificate_facts(&self.state).await;
        let last_error = crate::node::settings::acme_error(&self.state.database).await;
        let path = format!("/nodes/{}", node.id);

        layout::head(&node.name);
        let frame = Frame::new(&account, Area::Nodes, &projects, None, path);
        let body = rsx! {
                (layout::style_tag())
                <div class="split">
                    <div class="stack-sm">
                        <h1>(&node.name)</h1>
                        <p class="slug-preview">("wabot-deploy ")(node.version)</p>
                    </div>
                    <a class="btn btn-ghost" href="/nodes">("All nodes")</a>
                </div>

                (certificate_card(&facts, domain.as_deref(), last_error.as_deref(), &query))
                (memory_card(&snapshot))
                // The stream replaces the figures above in place.
                // Loaded last so the page is complete before it runs.
                (stream_script(&node.id))
        }
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }
}

/// Write the domain, treating a storage failure as a storage failure.
///
/// Separated only so the two call sites read as one line each; the
/// error is logged rather than returned because the redirect that
/// follows is the same either way.
async fn set_domain(database: &wabot::sqlite::SqliteDatabase, domain: Option<&str>) {
    if let Err(error) = crate::node::settings::set_domain(database, domain).await {
        tracing::error!(%error, "could not store the node's domain");
    }
}

/// What certificate this node serves, and how to change it.
///
/// The form is the answer to a node installed before its DNS was
/// ready: it serves its own authority's certificate, and until now
/// there was no way to ask again — with the same name or a different
/// one — without editing a file and restarting.
fn certificate_card<'a>(
    facts: &'a super::CertificateFacts,
    domain: Option<&'a str>,
    last_error: Option<&'a str>,
    query: &'a NodePage,
) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">("Certificate")</p>
                @if facts.trusted {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>("Trusted")
                    </span>
                } @else {
                    <span class="badge badge-warning">
                        <span class="dot dot-warning"></span>("Not trusted")
                    </span>
                }
            </div>

            <dl class="kv">
                <dt>("Domain")</dt>
                <dd>(domain.unwrap_or("not set"))</dd>
                <dt>("Issuer")</dt>
                <dd>(&facts.issuer)</dd>
            </dl>

            @if let Some(message) = &query.checked {
                <p class="note">(message)</p>
            }
            @if let Some(message) = &query.error {
                (layout::error_note(message))
            }
            @if let Some(failure) = last_error {
                <p class="failure">("The last attempt said: ")(failure)</p>
            }

            <form method="post" action="/nodes/certificate" class="stack">
                <label for="domain">("Domain")</label>
                <input id="domain" name="domain" type="text" class="mono"
                       value=(domain.unwrap_or_default())
                       placeholder="node.example.com">
                <p class="field-hint">(
                    "It must resolve to this node, and this node must be reachable on \
                     port 80 — that is what the challenge answers on. Both are checked \
                     before anything is requested."
                )</p>
                <div class="actions">
                    <button type="submit">("Request a certificate")</button>
                </div>
            </form>
        </section>
    }
}

/// The memory breakdown, and the bar above it.
///
/// Every figure carries a `data-cell` naming what it is, which is the
/// whole contract with the script: the server decides what a row means
/// and the client only writes text into the ones it recognises.
fn memory_card(snapshot: &Snapshot) -> impl Renderable + '_ {
    let containers = snapshot.containers_total();

    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">("Memory")</p>
                <span class="mono" data-cell="summary">
                    (memory::human(snapshot.used()))(" of ")(memory::human(snapshot.total))
                </span>
            </div>

            <div class="meter" data-meter>
                <span class="meter-part meter-node"
                      style=(width(snapshot, snapshot.node)) data-bar="node"></span>
                <span class="meter-part meter-runtime"
                      style=(width(snapshot, snapshot.containerd + snapshot.shims))
                      data-bar="runtime"></span>
                <span class="meter-part meter-containers"
                      style=(width(snapshot, containers)) data-bar="containers"></span>
                <span class="meter-part meter-rest"
                      style=(width(snapshot, snapshot.rest())) data-bar="rest"></span>
            </div>

            <table class="mem">
                <tbody>
                    (row("node", "wabot-deploy", "meter-node", snapshot.node,
                         "The console, the edge and the deploy path — this process."))
                    (row("containerd", "containerd", "meter-runtime", snapshot.containerd,
                         "The container runtime, shared by every service."))
                    (row("shims", &format!("shims ({})", snapshot.shim_count), "meter-runtime",
                         snapshot.shims,
                         "One per running container. The runtime's overhead, not the image's."))
                    (row("containers", "containers", "meter-containers", containers,
                         "What the images themselves are using, from their cgroups."))
                    (row("rest", "everything else", "meter-rest", snapshot.rest(),
                         "The kernel, the distribution, and anything else on this machine."))
                </tbody>
            </table>

            <p class="note">(
                "The parts overlap slightly: a container's page cache counts both in its \
                 own reading and in the system's, and shared pages count for each process \
                 that maps them. \"Everything else\" is what is left over rather than a \
                 measurement of its own."
            )</p>

            @if snapshot.swap_total > 0 {
                <dl class="kv">
                    <dt>("Swap")</dt>
                    <dd data-cell="swap">
                        (memory::human(snapshot.swap_used))(" of ")
                        (memory::human(snapshot.swap_total))
                    </dd>
                </dl>
            }
        </section>
    }
}

fn row<'a>(
    key: &'a str,
    label: &'a str,
    swatch: &'a str,
    bytes: u64,
    detail: &'a str,
) -> impl Renderable + 'a {
    rsx! {
        <tr>
            <td class="mem-key">
                <span class=(format!("swatch {swatch}"))></span>
                (label)
            </td>
            <td class="mono" data-cell=(key)>(memory::human(bytes))</td>
            <td class="tile-detail">(detail)</td>
        </tr>
    }
}

fn width(snapshot: &Snapshot, bytes: u64) -> String {
    format!("width:{:.2}%", snapshot.percent_of_total(bytes))
}

/// The client half of the live reading.
///
/// Inline rather than a file: it is twenty lines, it belongs to this
/// one view, and a separate request for it would be a round trip to
/// learn what the page already knows.
fn stream_script(node_id: &str) -> impl Renderable + '_ {
    // XSS SAFETY: `node_id` is matched against the node list before
    // this renders — it is `local` or the page redirected — so nothing
    // from a request reaches this script.
    let script = format!(
        r#"
const source = new EventSource("/nodes/{node_id}/memory");
source.onmessage = (event) => {{
  const data = JSON.parse(event.data);
  for (const [key, value] of Object.entries(data.cells)) {{
    const cell = document.querySelector(`[data-cell="${{key}}"]`);
    if (cell) cell.textContent = value;
  }}
  for (const [key, value] of Object.entries(data.bars)) {{
    const bar = document.querySelector(`[data-bar="${{key}}"]`);
    if (bar) bar.style.width = value;
  }}
}};
"#
    );
    rsx! { <script type="module">(hypertext::Raw::dangerously_create(&script))</script> }
}

/// The server half.
#[injectable]
pub struct NodeApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl NodeApi {
    /// Set the node's domain and ask for a certificate now.
    ///
    /// The name is checked against DNS first, exactly as a service
    /// hostname is: asking a certificate authority to validate a name
    /// that does not point here spends one of five hourly attempts to
    /// be told what a lookup would have said for free.
    #[post("/nodes/certificate")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_certificate(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !account.is_admin() {
            return Ok(super::auth::see_other("/"));
        }
        let here = "/nodes/local";

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let typed = super::auth::field(&form, "domain");

        let previous =
            crate::node::settings::domain(&self.state.database, &self.state.config).await;

        if typed.is_empty() {
            set_domain(&self.state.database, None).await;
            self.forget(previous.as_deref(), None).await;
            return Ok(super::auth::back_with_error(
                here,
                "the domain was cleared — this node serves its own certificate",
            ));
        }
        let domain = crate::platform::ports::normalize_hostname(typed);

        // Resolved against itself: the node has no other way to know
        // where it is reachable, and "does this name arrive here" is
        // the question. See `deploy::dns`.
        let outcome = crate::deploy::dns::resolves_here(&domain, &domain).await;
        if !outcome.ok() {
            return Ok(super::auth::back_with_error(
                here,
                &outcome.explain(&domain),
            ));
        }

        set_domain(&self.state.database, Some(&domain)).await;
        // The new name has to reach the console, the old one has to
        // stop: the edge answers from the route table, and a rename
        // that only changed the setting would leave the console on a
        // name nobody asked for and off the one they did.
        self.forget(previous.as_deref(), Some(&domain)).await;

        Ok(super::auth::see_other(&format!(
            "{here}?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair(
                    "checked",
                    &format!(
                        "{domain} resolves here. The certificate is being requested — \
                         reload in a few seconds."
                    )
                )
                .finish()
        )))
    }

    /// Drop the name the node used to answer to, then rebuild the
    /// routes — which is also what asks for the certificate, since the
    /// two answer for the same names.
    async fn forget(&self, previous: Option<&str>, now: Option<&str>) {
        if let Some(previous) = previous.filter(|old| Some(*old) != now) {
            if let Err(error) =
                crate::edge::routes::forget_control_plane(&self.state.database, previous).await
            {
                tracing::warn!(%error, "could not drop the old console route");
            }
        }
        self.state.deployer.sync_routes().await;
    }

    /// A reading every two seconds, as server-sent events.
    ///
    /// `#[raw]`, because the body is a stream that never ends and the
    /// JSON path buffers a value. The stream stops when the browser
    /// disconnects — the send fails, and the task ends with it.
    #[get("/nodes/:node/memory")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn memory_stream(&self, request: Request) -> RestResult<Response> {
        if !signed_in(&self.auth).is_some_and(|account| account.is_admin()) {
            // A stream is not a page, so this is a status rather than a
            // redirect: an EventSource cannot follow one usefully.
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        }

        let path = request.uri().path().to_string();
        let id = super::auth::segments(&path)
            .get(1)
            .map(|id| id.to_string())
            .unwrap_or_default();
        let known = crate::node::settings::domain(&self.state.database, &self.state.config).await;
        if crate::node::find(known, &id).is_none() {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        }

        let deployer = self.state.deployer.clone();
        let stream = async_stream::stream! {
            loop {
                let snapshot = deployer.memory().await;
                let payload = serde_json::to_string(&cells(&snapshot))
                    .unwrap_or_else(|_| "{}".into());
                yield Ok::<_, std::convert::Infallible>(
                    wabot::rest::axum::body::Bytes::from(format!("data: {payload}\n\n")),
                );
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        };

        Ok(Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            // Every hop in between has to be told, or a proxy holds the
            // stream until it has "enough" and the page never updates.
            .header(header::CACHE_CONTROL, "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(stream))
            .expect("a constant response is well-formed"))
    }
}

/// What the script writes where.
///
/// Built here rather than in the browser so the two halves cannot
/// disagree about formatting: the same `human()` renders the first
/// paint and every update after it.
#[derive(serde::Serialize)]
struct Cells {
    cells: BTreeMap<String, String>,
    bars: BTreeMap<String, String>,
}

fn cells(snapshot: &Snapshot) -> Cells {
    let containers = snapshot.containers_total();
    let runtime = snapshot.containerd + snapshot.shims;

    let cells = BTreeMap::from([
        (
            "summary".to_string(),
            format!(
                "{} of {}",
                memory::human(snapshot.used()),
                memory::human(snapshot.total)
            ),
        ),
        ("node".to_string(), memory::human(snapshot.node)),
        ("containerd".to_string(), memory::human(snapshot.containerd)),
        ("shims".to_string(), memory::human(snapshot.shims)),
        ("containers".to_string(), memory::human(containers)),
        ("rest".to_string(), memory::human(snapshot.rest())),
        (
            "swap".to_string(),
            format!(
                "{} of {}",
                memory::human(snapshot.swap_used),
                memory::human(snapshot.swap_total)
            ),
        ),
    ]);

    let bars = BTreeMap::from([
        ("node".to_string(), width(snapshot, snapshot.node)),
        ("runtime".to_string(), width(snapshot, runtime)),
        ("containers".to_string(), width(snapshot, containers)),
        ("rest".to_string(), width(snapshot, snapshot.rest())),
    ]);

    Cells { cells, bars }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    fn snapshot() -> Snapshot {
        Snapshot {
            total: 1024 * 1024 * 1024,
            available: 512 * 1024 * 1024,
            node: 12 * 1024 * 1024,
            containerd: 30 * 1024 * 1024,
            shims: 11 * 1024 * 1024,
            shim_count: 1,
            containers: BTreeMap::from([("demo.web".into(), 13 * 1024 * 1024)]),
            ..Default::default()
        }
    }

    /// The contract between the two halves: every cell the script
    /// writes has to exist in the page it writes into, or the first
    /// update silently does nothing.
    #[test]
    fn every_streamed_cell_has_a_place_on_the_page() {
        let snapshot = snapshot();
        let rendered = memory_card(&snapshot).render().into_inner();
        let payload = cells(&snapshot);

        for key in payload.cells.keys() {
            // Swap is only rendered when the machine has some.
            if key == "swap" {
                continue;
            }
            assert!(
                rendered.contains(&format!(r#"data-cell="{key}""#)),
                "nothing on the page shows {key}:\n{rendered}"
            );
        }
        for key in payload.bars.keys() {
            assert!(
                rendered.contains(&format!(r#"data-bar="{key}""#)),
                "no bar for {key}"
            );
        }
    }

    /// The figures are formatted once, on the server, so the first
    /// paint and every update after it read the same.
    #[test]
    fn the_first_paint_and_the_stream_agree() {
        let snapshot = snapshot();
        let rendered = memory_card(&snapshot).render().into_inner();

        for value in cells(&snapshot).cells.values() {
            if value.contains("of 0 B") {
                continue; // swap, which this machine does not have
            }
            assert!(
                rendered.contains(value.as_str()),
                "the page does not show {value}"
            );
        }
    }

    #[tokio::test]
    async fn the_list_shows_this_node() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/nodes")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Nodes"), "{body}");
        assert!(body.contains("/nodes/local"), "the node links to itself");
    }

    #[tokio::test]
    async fn a_node_page_shows_the_breakdown() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/nodes/local")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        for label in ["wabot-deploy", "containerd", "shims", "containers"] {
            assert!(body.contains(label), "{label} is missing from:\n{body}");
        }
    }

    #[tokio::test]
    async fn an_unknown_node_goes_back_to_the_list() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .get("/nodes/somewhere-else")
            .header("cookie", &cookie)
            .send()
            .await;
        response.assert_status(StatusCode::FOUND);
        assert_eq!(response.header("location"), Some("/nodes"));
    }

    /// The point of the whole form: a node installed before its DNS
    /// was ready serves its own certificate, and there has to be a way
    /// back from that without editing a file and restarting.
    #[tokio::test]
    async fn the_node_page_offers_to_request_a_certificate() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/nodes/local")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Request a certificate"), "{body}");
        assert!(
            body.contains("/nodes/certificate"),
            "the form posts somewhere"
        );
    }

    /// A name that does not point here cannot be accepted: asking an
    /// authority to validate it spends one of five hourly attempts to
    /// be told what a lookup says for free.
    #[tokio::test]
    async fn a_name_that_does_not_resolve_here_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .post("/nodes/certificate")
            .header("cookie", &cookie)
            // `.invalid` is reserved by RFC 2606 precisely so it can
            // never resolve — no network needed to know the answer.
            .form(&[("domain", "nowhere.invalid")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").unwrap_or_default();
        assert!(location.starts_with("/nodes/local?error="), "{location}");

        assert_eq!(
            crate::node::settings::domain(&console.database, &Config::default()).await,
            None,
            "a refused name must not be stored"
        );
    }

    /// Clearing it is how somebody goes back to a node with no public
    /// name — and it must not be gated on a lookup.
    #[tokio::test]
    async fn the_domain_can_be_cleared() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        crate::node::settings::set_domain(&console.database, Some("was.example"))
            .await
            .expect("set");

        console
            .harness
            .post("/nodes/certificate")
            .header("cookie", &cookie)
            .form(&[("domain", "")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert_eq!(
            crate::node::settings::domain(&console.database, &Config::default()).await,
            None
        );
    }

    /// Changing what the whole node answers to is not something a
    /// member of one project gets to do.
    #[tokio::test]
    async fn only_an_admin_may_change_it() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        crate::node::settings::set_domain(&console.database, Some("was.example"))
            .await
            .expect("set");

        let response = console
            .harness
            .post("/nodes/certificate")
            .header("cookie", &member)
            .form(&[("domain", "")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
        assert_eq!(
            crate::node::settings::domain(&console.database, &Config::default())
                .await
                .as_deref(),
            Some("was.example"),
            "a member changed the node's domain"
        );
    }

    /// The stream is as private as the page it feeds.
    #[tokio::test]
    async fn the_stream_needs_a_session() {
        let console = Console::new().await;
        console.signed_in().await;

        let response = console.harness.get("/nodes/local/memory").send().await;
        response.assert_status(StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_nodes_pages_need_a_session() {
        let console = Console::new().await;
        console.signed_in().await;

        for path in ["/nodes", "/nodes/local"] {
            let response = console.harness.get(path).send().await;
            assert_eq!(
                response.header("location"),
                Some("/sign-in"),
                "{path} was served to nobody"
            );
        }
    }
}
