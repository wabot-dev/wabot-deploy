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

        let nodes = crate::node::all(&self.state.config);
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
        let Some(node) = crate::node::find(&self.state.config, &query.node) else {
            return Ok(Redirect::found("/nodes").into());
        };

        let projects = access::projects_for(&self.state.database, &account).await?;
        let snapshot = self.state.deployer.memory().await;
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
        if crate::node::find(&self.state.config, &id).is_none() {
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
