//! Nodes: the list, and what one is spending.
//!
//! ## The only JavaScript in the product
//!
//! Two things on this page change without anybody pressing anything.
//! Memory changes every second, and a page showing a number from when
//! it was rendered is a page somebody reloads to check. A certificate
//! request finishes minutes after it starts, and the page used to
//! handle that by telling the operator to reload in a few seconds —
//! which is the console admitting it does not know.
//!
//! So this one view carries a script: an `EventSource` that replaces
//! both in place. One stream, not two, because one page holding two
//! connections open is twice the thing to get wrong; memory rides a
//! two-second tick and the certificate rides a change signal from the
//! renewal loop, and `live_stream` merges them.
//!
//! It is deliberately the *whole* of the client's involvement. The
//! page renders complete and correct with scripting off — the script
//! only replaces text and classes that are already there — so the
//! console keeps working on a machine where the stream cannot be
//! opened, which is the machine somebody is most likely looking at
//! this page from. Without it the page is a snapshot, which is what it
//! always was.

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

use crate::network::{self, enrolment::Enrolment, Kind};
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

#[derive(Debug, Deserialize, Validate)]
pub struct NodesPage {
    pub error: Option<String>,
    /// Names a join token this node is holding for one read. Not the
    /// token — see `ConsoleState::reveals`.
    pub shown: Option<String>,
    /// The node this one has just joined. A name, not a secret, so the
    /// query string is the right place for it — the same rule
    /// `back_with_error` follows.
    pub joined: Option<String>,
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
    async fn index(&self, query: NodesPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        // The node belongs to whoever runs it. Hiding the link is
        // courtesy; this is the boundary.
        if !account.is_admin() {
            return Ok(Redirect::found("/").into());
        }

        let nodes = network::all(&self.state.database).await?;
        let enrolments = network::enrolment::all(&self.state.database).await?;
        let authorities = network::authorities(&self.state.database).await?;
        let projects = access::projects_for(&self.state.database, &account).await?;
        // Spent on read: the token exists in clear for exactly this one
        // page load, and a query parameter would put it in the address
        // bar, the history and every refresh. See `ConsoleState::reveals`.
        let revealed = query
            .shown
            .as_deref()
            .and_then(|nonce| self.state.reveals.take(nonce));
        // Whether this node has an address another one could be told to
        // call back on. Not a setting — see `network::Node::may_be_edge`.
        let may_enrol = nodes.iter().any(|node| node.is_self && node.may_be_edge());
        let now = super::now_ms();

        layout::head("Nodes");
        let frame = Frame::new(&account, Area::Nodes, &projects, None, "/nodes");
        let body = rsx! {
                (layout::style_tag())
                <h1>("Nodes")</h1>
                <p class="tagline">(
                    "This node, and the ones that have agreed to take instructions \
                     from it."
                )</p>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }

                <div class="grid">
                    @for node in &nodes {
                        <a class="card tile" href=(format!("/nodes/{}", node.id))>
                            <div class="split">
                                <p class="tile-name">(&node.name)</p>
                                @if node.is_self {
                                    <span class="badge badge-success">
                                        <span class="dot dot-success"></span>("Serving")
                                    </span>
                                } @else {
                                    // Not "up": nothing has asked it
                                    // anything, so the only fact here is
                                    // that it joined. A green dot would
                                    // be this page inventing a health
                                    // check it does not run.
                                    <span class="badge badge-info">
                                        <span class="dot dot-info"></span>("Joined")
                                    </span>
                                }
                            </div>
                            <p class="tile-detail">
                                @if node.is_self {
                                    ("wabot-deploy ")(crate::api::VERSION)(" · this node")
                                    @if let Some(address) = &node.overlay_ip {
                                        (" · ")(address)
                                    }
                                } @else {
                                    (reach(node))
                                }
                            </p>
                        </a>
                    }
                </div>

                @if let Some(token) = &revealed {
                    <section class="card stack">
                        <p class="card-label">("Run this on the other node")</p>
                        <pre><code>("wabot-deploy join ")(token)</code></pre>
                        <p class="field-hint">(
                            "It works once and expires in 24 hours. This node will not \
                             show it again — what is stored is its hash. The other \
                             machine has to be a node already: install it there first, \
                             then join."
                        )</p>
                    </section>
                }

                // Ordered by which one this node is for. A node that can
                // be an edge is somebody's hub and wants to add nodes
                // below it; a node that cannot is one somebody is
                // pasting a token into, and burying that under a
                // paragraph about a feature it does not have would put
                // the only action it has second.
                @if may_enrol {
                    (enrol_card(true))
                    (join_card(query.joined.as_deref(), &enrolments, now))
                } @else {
                    (join_card(query.joined.as_deref(), &enrolments, now))
                    (enrol_card(false))
                }

                @if !authorities.is_empty() {
                    <section class="stack">
                        <p class="card-label">("Takes instructions from")</p>
                        <p class="field-hint">(
                            "Nodes this one has granted authority to. Revoking is one \
                             row, and it takes effect here — a node that has been \
                             revoked can ask for nothing."
                        )</p>
                        <table>
                            <thead>
                                <tr><th>("Node")</th><th>("State")</th><th></th></tr>
                            </thead>
                            <tbody>
                                @for authority in &authorities {
                                    (authority_row(authority, &nodes))
                                }
                            </tbody>
                        </table>
                    </section>
                }
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
        let Some(node) = network::find(&self.state.database, &query.node).await? else {
            return Ok(Redirect::found("/nodes").into());
        };

        let projects = access::projects_for(&self.state.database, &account).await?;

        // A node that is not this one has no memory reading and no
        // certificate: both are answers only the machine itself can
        // give, and nothing asks it anything yet. So the page says what
        // is known rather than rendering cards full of "unknown".
        if !node.is_self {
            let path = format!("/nodes/{}", node.id);
            // Everything this node could ask that one to run, named the
            // way somebody picking from a list would recognise.
            let projects_here = crate::platform::projects::all(&self.state.database).await?;
            let hostable: Vec<(String, String)> =
                crate::platform::services::all(&self.state.database, None)
                    .await?
                    .into_iter()
                    .map(|service| {
                        let project = projects_here
                            .iter()
                            .find(|project| project.id == service.project_id)
                            .map(|project| project.name.as_str())
                            .unwrap_or("?");
                        (service.id.clone(), format!("{project} · {}", service.name))
                    })
                    .collect();
            let orders: Vec<network::errand::Record> = network::errand::all(&self.state.database)
                .await?
                .into_iter()
                .filter(|record| record.node_id == node.id)
                .collect();
            layout::head(&node.name);
            let frame = Frame::new(&account, Area::Nodes, &projects, None, path);
            let body = rsx! {
                    (layout::style_tag())
                    <div class="split">
                        <div class="stack-sm">
                            <h1>(&node.name)</h1>
                            <p class="slug-preview">("joined · ")(reach(&node))</p>
                        </div>
                        <a class="btn btn-ghost" href="/nodes">("All nodes")</a>
                    </div>
                    @if let Some(message) = &query.error {
                        (layout::error_note(message))
                    }
                    (network_card(&node))

                    <section class="stack">
                        <p class="card-label">("Run a service there")</p>
                        @if hostable.is_empty() {
                            <section class="card stack">
                                <p>(
                                    "This node has no services to send. Deploy one here \
                                     first — what travels is an instruction to run the \
                                     same image, pulled from this node's registry."
                                )</p>
                            </section>
                        } @else {
                            <form method="post"
                                  action=(format!("/nodes/{}/host", node.id))
                                  class="card stack">
                                <label for="service">("Service")</label>
                                <select id="service" name="service">
                                    @for (id, label) in &hostable {
                                        <option value=(id)>(label)</option>
                                    }
                                </select>
                                <p class="field-hint">(
                                    "That node writes its own project, its own service \
                                     row and its own deployment — nothing is shared. It \
                                     pulls the image from this node's registry with a \
                                     credential this puts in the instruction, so the \
                                     image travels only when it is needed."
                                )</p>
                                <p class="field-hint">(
                                    "Nothing routes to it yet: telling an edge to serve \
                                     a name from there is the next piece. This runs the \
                                     container."
                                )</p>
                                <div class="actions">
                                    <button type="submit">("Ask it to run this")</button>
                                </div>
                            </form>
                        }
                        @if !orders.is_empty() {
                            <table>
                                <thead>
                                    <tr>
                                        <th>("Asked")</th><th>("State")</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    @for order in &orders {
                                        (errand_row(order))
                                    }
                                </tbody>
                            </table>
                        }
                    </section>

                    <section class="card stack">
                        <p class="card-label">("Forget this node")</p>
                        <p class="field-hint">(
                            "This node stops listing it. It is one direction only: the \
                             other node still holds this one as an authority until \
                             somebody revokes it there, from its own console. A grant \
                             belongs to the node that made it."
                        )</p>
                        <form method="post" action=(format!("/nodes/forget/{}", node.id))>
                            <button class="btn btn-ghost destructive" type="submit">
                                ("Forget")
                            </button>
                        </form>
                    </section>
            }
            .render()
            .into_inner();

            return Ok(frame.render(body).into_view().into());
        }

        let snapshot = self.state.deployer.memory().await;
        let facts = super::certificate_facts(&self.state).await;
        let policy = crate::edge::policy::for_name(
            &self.state.database,
            &self.state.config,
            domain
                .as_deref()
                .unwrap_or(crate::edge::certs::FALLBACK_NAME),
        )
        .await;
        let state = CertificateState::read(
            &facts,
            domain.as_deref(),
            // This name's own, falling back to the node-wide reason for
            // a failure recorded before there were per-name ones.
            policy
                .last_error
                .clone()
                .or(crate::node::settings::acme_error(&self.state.database).await),
            self.state.certificates.phase(),
            self.state.config.acme.disabled,
        );
        let cells = certificate_cells(&state, &facts, domain.as_deref());
        let policy = crate::edge::policy::for_name(
            &self.state.database,
            &self.state.config,
            domain
                .as_deref()
                .unwrap_or(crate::edge::certs::FALLBACK_NAME),
        )
        .await;
        let path = format!("/nodes/{}", node.id);

        layout::head(&node.name);
        let frame = Frame::new(&account, Area::Nodes, &projects, None, path);
        let body = rsx! {
                (layout::style_tag())
                <div class="split">
                    <div class="stack-sm">
                        <h1>(&node.name)</h1>
                        <p class="slug-preview">("wabot-deploy ")(crate::api::VERSION)</p>
                    </div>
                    <a class="btn btn-ghost" href="/nodes">("All nodes")</a>
                </div>

                // One island around both cards: the stream replaces
                // figures in each, and the runtime tears it down when
                // this host leaves the DOM. It used to be an inline
                // `<script>`, which a boosted navigation never runs —
                // arriving here by clicking a link left the page a
                // snapshot with no sign that it was one.
                (live_cards(&node.id, &cells, &state, &policy, domain.as_deref(), &query, &snapshot))

                // Outside the island: none of it streams, and a card
                // inside the host would be replaced by an update that
                // has nothing to say about it.
                (network_card(&node))
        }
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }
}

/// Mint a token for a node that is not here yet — or say why not.
///
/// The refusal is a card rather than a hidden form: "this node cannot
/// do that" is an answer, and a section that vanishes leaves somebody
/// looking for the thing they read about.
fn enrol_card(may_enrol: bool) -> impl Renderable {
    rsx! {
        <section class="stack">
            <p class="card-label">("Add a private node")</p>
            @if may_enrol {
                <form method="post" action="/nodes/enrol" class="card stack">
                    <label for="name">("What to call it")</label>
                    <input id="name" name="name" type="text"
                           placeholder="alpine" required>
                    <p class="field-hint">(
                        "This mints a token carrying this node's address, its overlay \
                         key and an address on the overlay for the new node. Joining \
                         with it there records this node as an authority — which that \
                         node can revoke at any time — and tells this one it arrived."
                    )</p>
                    // The failure that will actually happen, said beside
                    // the button rather than discovered on the far
                    // machine: joining calls back over this hostname,
                    // and a node serving its own authority's certificate
                    // is one nothing else trusts yet.
                    <p class="field-hint">(
                        "The other machine has to trust the certificate this console \
                         is served on. Until this node has a public one, joining will \
                         refuse rather than send its token to whatever answered."
                    )</p>
                    <div class="actions">
                        <button type="submit">("Create join token")</button>
                    </div>
                </form>
            } @else {
                <section class="card stack">
                    <p>(
                        "This node has no address another one could call back on, so it \
                         cannot enrol anybody yet."
                    )</p>
                    <p class="field-hint">(
                        "Set a domain on this node's own page. A joining node reaches it \
                         over the same hostname and certificate this console is served \
                         on, and that certificate has to be one the other machine \
                         already trusts."
                    )</p>
                </section>
            }
        </section>
    }
}

/// Paste a token from somewhere else, and what became of the ones this
/// node minted.
///
/// One section, because they are two halves of the same question — the
/// tokens this node handed out, and the one it was handed.
fn join_card<'a>(
    joined: Option<&'a str>,
    enrolments: &'a [Enrolment],
    now: i64,
) -> impl Renderable + 'a {
    rsx! {
        <section class="stack">
            <p class="card-label">("Join a network")</p>
            @if let Some(name) = joined {
                <section class="card stack">
                    <p>("This node now takes instructions from ")(name)(".")</p>
                    <p class="field-hint">(
                        "Nothing travels yet — errands need the overlay. It is listed \
                         below, and revoking it there takes effect here and \
                         immediately."
                    )</p>
                </section>
            }
            <form method="post" action="/nodes/join" class="card stack">
                <label for="token">("Join token")</label>
                <input id="token" name="token" type="text" class="mono"
                       placeholder="wdj1.…" autocomplete="off" required>
                <p class="field-hint">(
                    "From the nodes page of the node you are joining: add a private \
                     node there, and it shows one token, once. Pasting it here does \
                     what `wabot-deploy join` does in a terminal — this node records \
                     that one as an authority and tells it so. The same token can be \
                     pasted again if something goes wrong part-way."
                )</p>
                <div class="actions">
                    <button type="submit">("Join")</button>
                </div>
            </form>

            @if !enrolments.is_empty() {
                <p class="card-label">("Tokens this node minted")</p>
                <table>
                    <thead>
                        <tr>
                            <th>("For")</th><th>("Address")</th>
                            <th>("State")</th><th></th>
                        </tr>
                    </thead>
                    <tbody>
                        @for enrolment in enrolments {
                            (enrolment_row(enrolment, now))
                        }
                    </tbody>
                </table>
            }
        </section>
    }
}

/// What a node is, in the one word the list has room for.
fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Public => "public",
        Kind::Private => "private",
    }
}

/// How this node reaches another one — or, for itself, what it is.
///
/// Two questions, and they were being answered by one column. A node
/// with a public address that joins a hub reads `public` on its own
/// page, because it is, and the hub had it listed as `private` at the
/// same moment. Both rows were right and the pair was nonsense: the
/// hub does not know what that machine is, only that it has nothing
/// but an overlay address for it.
///
/// So a row about somebody else describes the *relationship*. Found by
/// looking at the two consoles side by side, which is the only place
/// it is visible.
fn reach(node: &network::Node) -> String {
    if node.is_self {
        return kind_word(node.kind).to_string();
    }
    match &node.overlay_ip {
        // Not "reached over the overlay": nothing is, yet. Where it
        // lives, which is all that has actually been settled.
        Some(address) => format!("on the overlay at {address}"),
        None => "no address for it yet".to_string(),
    }
}

/// What is known about a node's place on the overlay.
///
/// The same card for this node and for a joined one, because it is the
/// same set of facts: an id other nodes use, an address, a key, and
/// where this node would reach it. A node that has never enrolled
/// anybody and never joined anything has neither of the middle two, and
/// says so rather than showing an empty row.
fn network_card(node: &network::Node) -> impl Renderable + '_ {
    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">("On this network")</p>
                <span class="badge badge-info">
                    <span class="dot dot-info"></span>(reach(node))
                </span>
            </div>

            <dl class="kv">
                <dt>("Id")</dt>
                <dd class="mono">(&node.id)</dd>
                // "Reachable at" and not "endpoint": the empty case is
                // the interesting one, and it means this node has no
                // address to dial that one at — not that the machine
                // is unreachable, which is not something this node
                // knows. See `reach`.
                <dt>("This node dials it at")</dt>
                <dd class="mono">(node.endpoint.as_deref().unwrap_or("—"))</dd>
                <dt>("Overlay address")</dt>
                <dd class="mono">(node.overlay_ip.as_deref().unwrap_or("not on one"))</dd>
                <dt>("Public key")</dt>
                <dd class="mono">(node.public_key.as_deref().unwrap_or("none yet"))</dd>
            </dl>

            @if node.is_self {
                <p class="note">(
                    "A key and an address appear the first time this node enrols \
                     another one or joins one itself. Nothing travels over the overlay \
                     yet — an errand has no way to reach a node until there is a tunnel \
                     to carry it."
                )</p>
            } @else {
                <p class="note">(
                    "Recorded when this node joined — what it said about itself when it \
                     arrived. Instructions do not travel over the overlay: that node \
                     collects them over the same connection it enrolled through, which \
                     is why nothing here has to be able to reach it."
                )</p>
            }
        </section>
    }
}

/// One instruction, and what became of it.
///
/// A failure is an *answer*, and it says what it was — the state worth
/// worrying about is the one that never came back, which is why "asked
/// for and never settled" is its own word rather than an absence.
fn errand_row(order: &network::errand::Record) -> impl Renderable + '_ {
    rsx! {
        <tr>
            <td>
                (order.kind.as_str())
                <span class="tile-detail">(" ")(&order.id)</span>
            </td>
            <td>
                @if let Some(reason) = &order.error {
                    <span class="badge badge-danger">
                        <span class="dot dot-danger"></span>("Refused")
                    </span>
                    <p class="failure">(reason)</p>
                } @else if order.done() {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>("Done")
                    </span>
                } @else if order.taken_at.is_some() {
                    <span class="badge badge-info">
                        <span class="dot dot-info dot-pulse"></span>("Collected")
                    </span>
                } @else {
                    <span class="badge badge-info">
                        <span class="dot dot-info"></span>("Waiting to be collected")
                    </span>
                }
            </td>
        </tr>
    }
}

/// One join token, and what became of it.
fn enrolment_row(enrolment: &Enrolment, now: i64) -> impl Renderable + '_ {
    let spent = enrolment.spent();
    rsx! {
        <tr>
            <td>(&enrolment.name)</td>
            <td class="mono">(&enrolment.overlay_ip)</td>
            <td>
                // "Is it still worth carrying to a machine" first,
                // because it is the question somebody opens this table
                // to answer.
                @if enrolment.live(now) {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>("Waiting")
                    </span>
                } @else if spent {
                    <span class="badge">("Used")</span>
                } @else {
                    <span class="badge badge-warning">("Expired")</span>
                }
            </td>
            <td>
                // A used token is history, and withdrawing one would
                // free an address a node is already answering to.
                @if !spent {
                    <form method="post"
                          action=(format!("/nodes/enrolments/{}/withdraw", enrolment.id))>
                        <button class="btn btn-ghost destructive btn-sm" type="submit">
                            ("Withdraw")
                        </button>
                    </form>
                }
            </td>
        </tr>
    }
}

/// One authority, named by the node it belongs to when that node is
/// known here — an id is not something anybody recognises.
fn authority_row<'a>(
    authority: &'a network::Authority,
    nodes: &'a [network::Node],
) -> impl Renderable + 'a {
    let name = nodes
        .iter()
        .find(|node| node.id == authority.node_id)
        .map(|node| node.name.as_str())
        .unwrap_or(authority.node_id.as_str());

    rsx! {
        <tr>
            <td>
                (name)
                <span class="tile-detail">(" ")(&authority.node_id)</span>
            </td>
            <td>
                @if authority.live() {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>("Allowed")
                    </span>
                } @else {
                    <span class="badge">("Revoked")</span>
                }
            </td>
            <td>
                @if authority.live() {
                    <form method="post"
                          action=(format!("/nodes/revoke/{}", authority.node_id))>
                        <button class="btn btn-ghost destructive btn-sm" type="submit">
                            ("Revoke")
                        </button>
                    </form>
                }
            </td>
        </tr>
    }
}

/// Where this node's own page is.
///
/// Looked up rather than named by a constant: the id is minted when the
/// node is installed, so there is no path a `const` could hold. Back to
/// the list if there is somehow no row — a redirect to a page that does
/// not exist is worse than one to the page above it.
async fn self_path(database: &wabot::sqlite::SqliteDatabase) -> String {
    match network::me(database).await {
        Ok(Some(me)) => format!("/nodes/{}", me.id),
        Ok(None) => "/nodes".to_string(),
        Err(error) => {
            tracing::warn!(%error, "could not read this node's own row");
            "/nodes".to_string()
        }
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

/// What the node's TLS is, as one answer.
///
/// Assembled rather than read off a single column, because the honest
/// answer needs three sources: whether a public authority is being
/// asked at all, what is installed, and whether the loop is asking
/// right now. The page used to show only the last *failure*, so a node
/// that had just been given a domain looked identical to one that had
/// never been asked about — and the button said "Request a
/// certificate" whether or not a request was possible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CertificateState {
    /// ACME is off in the config. The local authority is the answer,
    /// not a stage on the way to one.
    Local,
    /// No domain, so there is no name a public authority could certify.
    Nameless,
    /// The loop is asking, now.
    Working,
    /// A domain is set, nothing has failed, and nothing has arrived.
    Waiting,
    Trusted,
    /// Real ACME against Let's Encrypt staging: untrusted by design.
    Staging,
    /// Read off disk. Whether a browser trusts it depends on a trust
    /// store this node cannot see, so it is reported as what it is
    /// rather than as trusted or not.
    FromFile,
    /// Asked and refused. The loop keeps trying.
    Failed(String),
}

impl CertificateState {
    pub(crate) fn read(
        facts: &super::CertificateFacts,
        domain: Option<&str>,
        last_error: Option<String>,
        phase: crate::edge::acme::Phase,
        acme_disabled: bool,
    ) -> Self {
        if acme_disabled {
            return Self::Local;
        }
        if domain.is_none() {
            return Self::Nameless;
        }
        // Before the outcome, on purpose: a renewal of a certificate
        // that is currently fine is still something happening, and a
        // page that hid it would be a page that never shows this state
        // on the node where it works.
        if phase == crate::edge::acme::Phase::Working {
            return Self::Working;
        }
        // Before the trust check, because trust is not a thing this
        // node can decide about a certificate it was handed.
        if facts.source == crate::edge::certs::Source::File {
            return Self::FromFile;
        }
        if facts.trusted {
            return Self::Trusted;
        }
        if facts.issuer == "letsencrypt-staging" {
            return Self::Staging;
        }
        match last_error {
            Some(reason) => Self::Failed(reason),
            None => Self::Waiting,
        }
    }

    /// The word, and the two classes that colour it.
    fn badge(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Local => ("Local", "badge badge-info", "dot dot-info"),
            Self::Nameless => ("Self-signed", "badge badge-info", "dot dot-info"),
            Self::Working => ("Requesting", "badge badge-info", "dot dot-info dot-pulse"),
            Self::Waiting => ("Waiting", "badge badge-info", "dot dot-info dot-pulse"),
            Self::Trusted => ("Trusted", "badge badge-success", "dot dot-success"),
            Self::Staging => ("Not trusted", "badge badge-warning", "dot dot-warning"),
            Self::FromFile => ("From file", "badge badge-info", "dot dot-info"),
            Self::Failed(_) => ("Failed", "badge badge-danger", "dot dot-danger"),
        }
    }

    /// What it means, in one sentence somebody can act on.
    fn note(&self) -> &'static str {
        match self {
            Self::Local => {
                "A public authority is not being asked for this node. It serves its \
                 own authority's certificate, which browsers will not trust."
            }
            Self::Nameless => {
                "This node has no domain, so there is no name an authority could \
                 certify. It serves its own authority's certificate. Set a domain \
                 below and it will ask for a public one."
            }
            Self::Working => {
                "Asking the certificate authority now. This page updates itself when \
                 the answer arrives."
            }
            Self::Waiting => {
                "The domain is set and the request has not been made yet. This page \
                 updates itself when it is."
            }
            Self::Trusted => {
                "This page reached you over TLS with a publicly trusted certificate \
                 the node obtained and installed on its own."
            }
            Self::Staging => {
                "Staging certificate — browsers will not trust it, which is expected. \
                 Set acme.directory to production when you are done testing."
            }
            Self::FromFile => {
                "Read from the files below. This node cannot renew it — it did not \
                 ask for it — so whatever keeps those files current is what keeps \
                 this certificate current. If they stop being refreshed, the node \
                 signs for this name itself rather than let it expire."
            }
            Self::Failed(_) => {
                "The last attempt was refused. The node keeps trying, waiting longer \
                 after each failure, and serves its own authority's certificate in \
                 the meantime."
            }
        }
    }

    fn failure(&self) -> &str {
        match self {
            Self::Failed(reason) => reason,
            _ => "",
        }
    }

    /// Whether asking again, without changing anything, could help.
    fn may_retry(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Waiting)
    }
}

/// Everything the card shows, formatted.
///
/// Built here rather than in the browser so the two halves cannot
/// disagree: the same values render the first paint and every update
/// after it. Same reason as [`Cells`].
#[derive(serde::Serialize)]
pub(crate) struct CertificateCells {
    domain: String,
    pub(crate) issuer: String,
    pub(crate) renews: String,
    pub(crate) word: &'static str,
    pub(crate) badge: &'static str,
    pub(crate) dot: &'static str,
    note: &'static str,
    failure: String,
}

pub(crate) fn certificate_cells(
    state: &CertificateState,
    facts: &super::CertificateFacts,
    domain: Option<&str>,
) -> CertificateCells {
    let (word, badge, dot) = state.badge();
    CertificateCells {
        domain: domain.unwrap_or("not set").to_string(),
        issuer: facts.issuer.clone(),
        // Always a row, even when nothing renews: the script only
        // replaces text that is already on the page, so a row that
        // appears only sometimes is a row that can never update.
        renews: match facts.renews_in_days() {
            Some(days) => format!("{days} days"),
            None => "—".into(),
        },
        word,
        badge,
        dot,
        note: state.note(),
        failure: state.failure().to_string(),
    }
}

/// What certificate this node serves, and how to change it.
///
/// The form is the answer to a node installed before its DNS was
/// ready: it serves its own authority's certificate, and until now
/// there was no way to ask again — with the same name or a different
/// one — without editing a file and restarting.
///
/// The button says what it does. It used to say "Request a
/// certificate", which was one of the things it did, only sometimes,
/// and never the thing an operator came here to do: the field beside
/// it is the node's domain, and saving it is what starts everything
/// else.
fn certificate_card<'a>(
    cells: &'a CertificateCells,
    state: &'a CertificateState,
    policy: &'a crate::edge::policy::Policy,
    domain: Option<&'a str>,
    query: &'a NodePage,
) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">("Certificate")</p>
                <span class=(cells.badge) data-cert="badge">
                    <span class=(cells.dot) data-cert="dot"></span>
                    <span data-cert="word">(cells.word)</span>
                </span>
            </div>

            <dl class="kv">
                <dt>("Domain")</dt>
                <dd data-cert="domain">(&cells.domain)</dd>
                <dt>("Issuer")</dt>
                <dd data-cert="issuer">(&cells.issuer)</dd>
                <dt>("Renews in")</dt>
                <dd data-cert="renews">(&cells.renews)</dd>
            </dl>

            <p class="note" data-cert="note">(cells.note)</p>
            // Always present, empty when there is nothing to say: the
            // script assigns into it, and `.failure:empty` hides it.
            <p class="failure" data-cert="failure">(&cells.failure)</p>

            @if let Some(message) = &query.checked {
                <p class="note">(message)</p>
            }
            @if let Some(message) = &query.error {
                (layout::error_note(message))
            }

            <form method="post" action="/nodes/certificate" class="stack">
                <label for="domain">("Domain")</label>
                <input id="domain" name="domain" type="text" class="mono"
                       value=(domain.unwrap_or_default())
                       placeholder="node.example.com">
                <p class="field-hint">(
                    "It must resolve to this node, and this node must be reachable on \
                     port 80 — that is what the challenge answers on. Both are checked \
                     before anything is requested. Saving reissues: the node takes the \
                     new name on its own certificate straight away, then asks a public \
                     authority for one."
                )</p>
                <div class="actions">
                    <button type="submit">("Save domain")</button>
                    @if state.may_retry() {
                        // Asking again with the same name is its own
                        // action: the usual fix for a refusal is out
                        // there — a DNS record, a firewall — and having
                        // to re-type the domain to retry reads as
                        // though the domain were the problem.
                        <button class="btn btn-secondary" type="submit"
                                name="retry" value="1">("Try again")</button>
                    }
                </div>
            </form>

            (certificate_source_form("/nodes/certificate/source", policy))
        </section>
    }
}

/// Where one name's certificate comes from.
///
/// Shared by the node's own certificate and every service hostname,
/// because it is the same question — the node's domain is not a
/// special case, it is only the name that had a page first. The ids
/// carry the action so two of these on one page do not collide.
pub(crate) fn certificate_source_form(
    action: &str,
    policy: &crate::edge::policy::Policy,
) -> impl Renderable {
    let (cert_path, key_path) = match &policy.renew_with {
        crate::edge::policy::RenewWith::File {
            cert_path,
            key_path,
        } => (cert_path.as_str(), key_path.as_str()),
        _ => ("", ""),
    };
    let field = |suffix: &str| format!("{}-{suffix}", policy.name);

    // An island host, not a bare form: the client runtime re-hydrates
    // registered islands after a boosted navigation, and a listener
    // attached once at load would belong to a form the next swap threw
    // away. Without scripting this is a `<wabot-island>` wrapper around
    // a form that works, which is the whole cost.
    wabot::ui::hypertext::island_bare(
        "fields",
        rsx! {
            <form method="post" action=(action) class="stack">
                <label for=(field("source"))>("Where the certificate comes from")</label>
                <select id=(field("source")) name="renew_with">
                    @for (value, label) in SOURCES {
                        @if policy.renew_with.as_str() == *value {
                            <option value=(value) selected>(label)</option>
                        } @else {
                            <option value=(value)>(label)</option>
                        }
                    }
                </select>

                // Only the file answer needs these, so only the file
                // answer shows them. Without scripting they stay visible
                // and the form still works — the server reads them only
                // when `renew_with` says `file`.
                <div class="stack" data-when="renew_with=file">
                    <label for=(field("cert"))>("Certificate file")</label>
                    <input id=(field("cert")) name="cert_path" type="text" class="mono"
                           value=(cert_path) placeholder="/etc/ssl/name.crt"
                           data-required-when="renew_with=file">
                    <label for=(field("key"))>("Key file")</label>
                    <input id=(field("key")) name="key_path" type="text" class="mono"
                           value=(key_path) placeholder="/etc/ssl/name.key"
                           data-required-when="renew_with=file">
                    <p class="field-hint">(
                        "Both are read now and refused if they do not match, do not cover \
                         this name, or have already expired — a bad pair installed would \
                         break every handshake, including the one serving this page. After \
                         that the node rereads them and reinstalls whatever it finds, which \
                         is how a certificate it cannot renew stays current."
                    )</p>
                </div>
                <div class="actions">
                    <button class="btn btn-secondary" type="submit">("Save source")</button>
                </div>
            </form>
        },
    )
}

/// The three answers, in the order somebody is likeliest to want them.
const SOURCES: &[(&str, &str)] = &[
    ("acme", "A public authority (Let's Encrypt)"),
    ("self_signed", "This node's own authority"),
    ("file", "Read from files on this node"),
];

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

/// The two cards that update themselves, in one island host.
///
/// Props carry the node id, because the client needs it to open the
/// stream and reading it out of the URL would be the browser deriving
/// something the server already knew.
#[allow(clippy::too_many_arguments)]
fn live_cards<'a>(
    node_id: &'a str,
    cells: &'a CertificateCells,
    state: &'a CertificateState,
    policy: &'a crate::edge::policy::Policy,
    domain: Option<&'a str>,
    query: &'a NodePage,
    snapshot: &'a Snapshot,
) -> impl Renderable + 'a {
    let inner = rsx! {
        (certificate_card(cells, state, policy, domain, query))
        (memory_card(snapshot))
    }
    .render()
    .into_inner();

    // Rendered first, then wrapped: `rsx!` expands to a closure that
    // captures by move, and nesting one inside the island's would have
    // both wanting the same borrows. Same reason as `Frame::render`.
    wabot::ui::hypertext::island(
        "node-live",
        &serde_json::json!({ "node": node_id }),
        hypertext::Raw::dangerously_create(&inner),
    )
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
        let here = &self_path(&self.state.database).await;

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let typed = super::auth::field(&form, "domain");

        let previous =
            crate::node::settings::domain(&self.state.database, &self.state.config).await;

        // "Try again" is the same form with nothing changed. It exists
        // because the usual fix for a refusal is somewhere else — a DNS
        // record, a firewall — and making somebody re-type the domain
        // to retry reads as though the domain were what was wrong.
        if form.contains_key("retry") {
            self.state.certificates.now();
            return Ok(super::auth::see_other(here));
        }

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
        // A name change invalidates whatever is being served: the old
        // certificate does not carry the new name. Clearing the last
        // failure is part of that — it was about the old name, and
        // leaving it would make the page report the new domain as
        // already refused before anything had been asked.
        if let Err(error) = crate::node::settings::set_acme_error(&self.state.database, None).await
        {
            tracing::warn!(%error, "could not clear the last certificate failure");
        }
        // The new name has to reach the console, the old one has to
        // stop: the edge answers from the route table, and a rename
        // that only changed the setting would leave the console on a
        // name nobody asked for and off the one they did.
        self.forget(previous.as_deref(), Some(&domain)).await;
        // The loop owns issuance — both the local certificate carrying
        // the new name and the public one. Asking it here is what makes
        // saving a domain reissue rather than merely record.
        self.state.certificates.now();

        // No "reload in a few seconds": the page watches. What it says
        // here is the one thing the stream cannot — that the name was
        // checked before anything was asked for.
        Ok(super::auth::see_other(&format!(
            "{here}?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("checked", &format!("{domain} resolves here."))
                .finish()
        )))
    }

    /// Mint a join token for a node that does not exist here yet.
    ///
    /// Everything the other machine needs is decided here, because
    /// everything it needs is this node's to decide: the address on
    /// this node's overlay, the key it will talk to, and the secret it
    /// will authenticate with. The token is shown once and then only
    /// its hash is kept.
    #[post("/nodes/enrol")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn enrol(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        // Enrolling a node is not a project-level decision: it grants
        // whoever holds the token a place on this node's network.
        if !account.is_admin() {
            return Ok(super::auth::see_other("/"));
        }
        let here = "/nodes";

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let name = super::auth::field(&form, "name");
        if name.is_empty() || name.chars().count() > 60 {
            return Ok(super::auth::back_with_error(
                here,
                "give the node a name — a list of unnamed tokens is unreadable",
            ));
        }

        // A key, a row and an address for this node, in that order, and
        // only now: a node that never enrols anybody needs none of them.
        let me = network::ensure_hub(&self.state.database, &self.state.config).await?;
        // Refused rather than minted into a token nothing can use: the
        // endpoint is what the other node calls back on, and a token
        // carrying no address is one that fails on the far machine
        // where nobody is watching.
        let Some(endpoint) = me.endpoint.clone().filter(|_| me.may_be_edge()) else {
            return Ok(super::auth::back_with_error(
                here,
                "this node has no address another one could call back on — set a domain first",
            ));
        };
        let Some(public_key) = network::keys::public_key(&self.state.database).await else {
            return Ok(super::auth::back_with_error(
                here,
                "this node has no overlay key yet",
            ));
        };
        let Some(overlay_ip) = me.overlay_ip.clone() else {
            return Ok(super::auth::back_with_error(
                here,
                "this node has no address on its own overlay yet",
            ));
        };

        let assigned_ip = network::overlay::allocate(&self.state.database).await?;
        let (_, secret) =
            network::enrolment::create(&self.state.database, name, &assigned_ip, &account.id)
                .await?;

        let token = network::token::JoinToken {
            authority: me.id,
            name: me.name,
            endpoint,
            public_key,
            overlay_ip,
            assigned_ip,
            secret,
        };

        // The nonce travels, not the token. This is the only moment it
        // exists in clear, and a query string is read by the address
        // bar, the history and every refresh — the same reason a push
        // token stopped travelling that way. See `ConsoleState::reveals`.
        let nonce = self.state.reveals.stash(token.encode());
        Ok(super::auth::see_other(&format!(
            "{here}?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("shown", &nonce)
                .finish()
        )))
    }

    /// Take instructions from the node that minted this token.
    ///
    /// The console's door onto the same work `wabot-deploy join` does,
    /// and the reason it exists: a node that is already installed and
    /// already answering is one somebody is looking at through a
    /// browser, and telling them to go and find its terminal for a
    /// one-field form is telling them to go somewhere else to do the
    /// thing they are already here for.
    ///
    /// Inline rather than queued, unlike a deployment. It is one
    /// request with a timeout, and its whole value is the answer: a
    /// join that failed has a reason, and a reason nobody sees is the
    /// failure this console exists to prevent.
    #[post("/nodes/join")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn join(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        // Granting another node authority over this one is the largest
        // thing anybody can do from this console.
        if !account.is_admin() {
            return Ok(super::auth::see_other("/"));
        }
        let here = "/nodes";

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let token = super::auth::field(&form, "token");
        if token.is_empty() {
            return Ok(super::auth::back_with_error(here, "paste the join token"));
        }

        // The token is never carried back — not into the query string,
        // not into the form's value. A refusal says what was wrong with
        // it, which is all somebody needs to paste it again.
        match network::join::join(&self.state.database, &self.state.config, token).await {
            Ok(joined) => Ok(super::auth::see_other(&format!(
                "{here}?{}",
                form_urlencoded::Serializer::new(String::new())
                    .append_pair("joined", &joined.authority.name)
                    .finish()
            ))),
            Err(error) => Ok(super::auth::back_with_error(here, &error.to_string())),
        }
    }

    /// Withdraw a token nobody has spent.
    #[post("/nodes/enrolments/:enrolment/withdraw")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn withdraw(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !account.is_admin() {
            return Ok(super::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        if let Some(id) = super::auth::segments(&path).get(2) {
            network::enrolment::withdraw(&self.state.database, id).await?;
        }
        Ok(super::auth::see_other("/nodes"))
    }

    /// Ask another node to run one of this node's services.
    ///
    /// The image reference is used as it stands: it already names this
    /// node's registry, because that is where the push landed. So the
    /// far node pulls the same bytes this one runs, rather than
    /// resolving a tag of its own and getting whatever is current
    /// there — which would be two nodes running "the same" service and
    /// disagreeing about what that is.
    #[post("/nodes/:node/host")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn host_there(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        // Putting work on another machine is not a project-level
        // decision.
        if !account.is_admin() {
            return Ok(super::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        let Some(node_id) = super::auth::segments(&path).get(1).map(|id| id.to_string()) else {
            return Ok(super::auth::see_other("/nodes"));
        };
        let here = format!("/nodes/{node_id}");

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let service_id = super::auth::field(&form, "service");

        let Some(service) = crate::platform::services::all(&self.state.database, None)
            .await?
            .into_iter()
            .find(|service| service.id == service_id)
        else {
            return Ok(super::auth::back_with_error(&here, "no such service"));
        };
        let Some(project) =
            crate::platform::projects::find(&self.state.database, &service.project_id).await?
        else {
            return Ok(super::auth::back_with_error(&here, "no such project"));
        };
        // The registry is the host in the reference, read by the same
        // rule the pull path reads it by — one copy, because two
        // diverged the first time and `alpine:3.23` came out as a
        // registry called `alpine` on port `3.23`. A service whose image
        // names no registry is one the far node can pull without
        // anything from here, and the credential below would be going
        // somewhere it does not belong.
        let Some(registry) = crate::platform::registry_credentials::host_of(&service.image) else {
            return Ok(super::auth::back_with_error(
                &here,
                "that image does not name a registry, so there is nothing to pull it from",
            ));
        };

        // A credential for that one project, minted for this errand. It
        // is a push token because that is what this registry reads, and
        // it is more than a pull needs — a read-only scope belongs in
        // the registry and is not this change.
        let (_, secret) = match crate::platform::tokens::create(
            &self.state.database,
            &project.id,
            &format!("errand to {node_id}"),
            &account.id,
        )
        .await
        {
            Ok(minted) => minted,
            Err(error) => return Ok(super::auth::back_with_error(&here, &error.to_string())),
        };

        let payload = serde_json::to_value(network::errand::Host {
            project: project.name.clone(),
            service: service.name.clone(),
            image: service.image.clone(),
            registry: registry.clone(),
            username: "errand".into(),
            secret,
            env: service.env.clone(),
            port: None,
            // One copy. This form predates placement and is the one on
            // the wrong page — the service's own page is where a
            // replica count is chosen.
            slots: vec![1],
        })
        .unwrap_or_default();

        network::errand::queue(
            &self.state.database,
            &node_id,
            network::errand::Kind::Host,
            &payload,
        )
        .await?;

        Ok(super::auth::see_other(&here))
    }

    /// Stop listing a node.
    #[post("/nodes/forget/:node")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn forget_node(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !account.is_admin() {
            return Ok(super::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        if let Some(id) = super::auth::segments(&path).get(2) {
            network::forget(&self.state.database, id).await?;
        }
        Ok(super::auth::see_other("/nodes"))
    }

    /// Stop taking instructions from a node.
    ///
    /// The half of joining that makes it not a one-way door. It takes
    /// effect here and nowhere else — the other node keeps whatever it
    /// recorded, and finds out by being refused.
    #[post("/nodes/revoke/:node")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revoke_authority(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !account.is_admin() {
            return Ok(super::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        if let Some(id) = super::auth::segments(&path).get(2) {
            network::revoke(&self.state.database, id).await?;
        }
        Ok(super::auth::see_other("/nodes"))
    }

    /// Choose where this name's certificate comes from.
    ///
    /// The file pair is read and checked *here*, before the choice is
    /// stored. Storing a policy that cannot work would make the
    /// renewal loop fail on every pass with the reason in the journal,
    /// and this console exists so that reasons do not live there. The
    /// worse case is the one this refuses outright: a certificate and
    /// key that do not match install cleanly and then break every
    /// handshake, including the one serving this page.
    #[post("/nodes/certificate/source")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_certificate_source(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !account.is_admin() {
            return Ok(super::auth::see_other("/"));
        }
        let here = &self_path(&self.state.database).await;

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        // The node's own name, or the fallback while it has none — the
        // same key the renewal loop will look this up under.
        let name = crate::node::settings::domain(&self.state.database, &self.state.config)
            .await
            .unwrap_or_else(|| crate::edge::certs::FALLBACK_NAME.to_string());

        let renew_with = match super::auth::field(&form, "renew_with") {
            "self_signed" => crate::edge::policy::RenewWith::SelfSigned,
            "file" => {
                let cert_path = super::auth::field(&form, "cert_path").trim().to_string();
                let key_path = super::auth::field(&form, "key_path").trim().to_string();
                if cert_path.is_empty() || key_path.is_empty() {
                    return Ok(super::auth::back_with_error(
                        here,
                        "reading from files needs both a certificate and a key",
                    ));
                }
                // Refused now, while there is somebody to tell.
                if let Err(error) = crate::edge::certs::from_files(&name, &cert_path, &key_path) {
                    return Ok(super::auth::back_with_error(here, &error.to_string()));
                }
                crate::edge::policy::RenewWith::File {
                    cert_path,
                    key_path,
                }
            }
            // Including anything a hand-written form might send: ACME
            // is the default, and defaulting to it cannot lose a
            // certificate the way defaulting to a file source could.
            _ => crate::edge::policy::RenewWith::Acme,
        };

        // Choosing the default *forgets* rather than writes it down.
        // A stored default would go stale the day `acme.disabled`
        // changed, and then say the opposite of what the node does.
        let stored =
            if renew_with == crate::edge::policy::RenewWith::default_for(&self.state.config) {
                crate::edge::policy::clear(&self.state.database, &name).await
            } else {
                crate::edge::policy::set(&self.state.database, &name, &renew_with).await
            };
        if let Err(error) = stored {
            return Ok(super::auth::back_with_error(here, &error.to_string()));
        }
        // The loop installs it. Same reason as saving a domain: this
        // console asks, it does not issue.
        self.state.certificates.now();

        Ok(super::auth::see_other(here))
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

    /// The node's live state, as server-sent events.
    ///
    /// One stream for both halves of this page rather than two. Memory
    /// wants a tick; a certificate wants to be told. Merging them keeps
    /// the console to a single `EventSource` — see the module docs —
    /// and means the certificate answer lands the moment the loop has
    /// it instead of up to two seconds later.
    ///
    /// `#[raw]`, because the body is a stream that never ends and the
    /// JSON path buffers a value. The stream stops when the browser
    /// disconnects — the send fails, and the task ends with it.
    #[get("/nodes/:node/live")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn live_stream(&self, request: Request) -> RestResult<Response> {
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
        // Only this node's own page has a stream. What it carries — a
        // memory reading and a certificate — are answers only the
        // machine itself can give, so a stream for a joined node would
        // be this one reporting its own figures under somebody else's
        // name.
        let mine = network::me(&self.state.database)
            .await
            .unwrap_or_default()
            .is_some_and(|me| me.id == id);
        if !mine {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        }

        let state = self.state.clone();
        let mut phase = state.certificates.watch();
        let stream = async_stream::stream! {
            loop {
                let snapshot = state.deployer.memory().await;
                let facts = super::certificate_facts(&state).await;
                let domain =
                    crate::node::settings::domain(&state.database, &state.config).await;
                let certificate = CertificateState::read(
                    &facts,
                    domain.as_deref(),
                    crate::edge::policy::for_name(
                        &state.database,
                        &state.config,
                        domain.as_deref().unwrap_or(crate::edge::certs::FALLBACK_NAME),
                    )
                    .await
                    .last_error
                    .or(crate::node::settings::acme_error(&state.database).await),
                    state.certificates.phase(),
                    state.config.acme.disabled,
                );
                let payload = serde_json::to_string(&Live {
                    inner: cells(&snapshot),
                    certificate: certificate_cells(&certificate, &facts, domain.as_deref()),
                })
                .unwrap_or_else(|_| "{}".into());
                yield Ok::<_, std::convert::Infallible>(
                    wabot::rest::axum::body::Bytes::from(format!("data: {payload}\n\n")),
                );

                // Whichever comes first. The tick is for memory, which
                // nothing announces; the watch is for the certificate,
                // which does.
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                    changed = phase.changed() => {
                        // The sender lives in the shared state, so this
                        // only ends if the whole node is going down.
                        if changed.is_err() { break; }
                    }
                }
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

/// The whole payload: the memory reading, and the certificate beside
/// it. Flattened so the script reads `data.cells` exactly as it did
/// when memory was all this stream carried.
#[derive(serde::Serialize)]
struct Live {
    #[serde(flatten)]
    inner: Cells,
    certificate: CertificateCells,
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
pub(crate) mod tests {
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
        assert!(
            body.contains(&console.node_path),
            "the node links to itself: {body}"
        );
    }

    #[tokio::test]
    async fn a_node_page_shows_the_breakdown() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get(&console.node_path)
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
    ///
    /// The button says "Save domain" now. It used to say "Request a
    /// certificate", which described one thing it sometimes did rather
    /// than the thing it always does — and on a node with no domain,
    /// which is this one, no request was possible at all.
    #[tokio::test]
    async fn the_node_page_offers_to_save_a_domain() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get(&console.node_path)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Save domain"), "{body}");
        assert!(
            !body.contains("Request a certificate"),
            "and no longer promises a request: {body}"
        );
        assert!(
            body.contains("/nodes/certificate"),
            "the form posts somewhere"
        );
    }

    /// Every `data-when` has to name a control that exists in the page
    /// it is on. A typo in the condition is the failure mode of a
    /// declarative island: nothing throws, the field is simply hidden
    /// forever or shown forever, and the page looks fine.
    pub(crate) fn conditions_name_real_controls(body: &str) {
        let mut checked = 0;
        for attribute in ["data-when=\"", "data-required-when=\""] {
            for (index, _) in body.match_indices(attribute) {
                let rest = &body[index + attribute.len()..];
                let condition = &rest[..rest.find('"').expect("a closed attribute")];
                let name = condition.split('=').next().expect("a name");
                assert!(
                    body.contains(&format!("name=\"{name}\"")),
                    "{condition} names no control on this page"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "no conditional fields on this page");
    }

    /// The bug this island replaced: an inline `<script>` in the view
    /// body never runs when a boosted navigation swaps it in with
    /// `innerHTML`, so arriving here by clicking a link left a page
    /// that looked live and was a snapshot. An island host is data the
    /// runtime re-hydrates after every swap.
    #[tokio::test]
    async fn the_node_page_still_comes_alive_when_it_is_navigated_to() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let fragment = console
            .ui
            .with_header("cookie", cookie)
            .navigate(&console.node_path)
            .await;

        let html = fragment.html();
        assert!(
            html.contains("data-island=\"node-live\""),
            "the fragment carries the host: {html}"
        );
        assert!(
            !html.contains("<script"),
            "and nothing that a swap would silently not run: {html}"
        );
    }

    /// The island hides; it does not build. Both paths are in the HTML
    /// the node produced, so the form works with scripting off — that
    /// is the whole of the rule this console kept when it stopped
    /// saying "no JavaScript".
    #[tokio::test]
    async fn the_certificate_paths_are_rendered_and_merely_hidden() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get(&console.node_path)
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            body.contains("name=\"cert_path\""),
            "server-rendered: {body}"
        );
        assert!(body.contains("name=\"key_path\""), "{body}");
        assert!(
            body.contains("data-when=\"renew_with=file\""),
            "and conditioned: {body}"
        );
        conditions_name_real_controls(&body);
    }

    /// A node with no domain has nothing a public authority could
    /// certify, and that is a *state*, not a failure. It used to be
    /// indistinguishable from a node whose request had been refused,
    /// because only failures were recorded.
    #[tokio::test]
    async fn a_node_with_no_domain_says_so_rather_than_nothing() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get(&console.node_path)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Self-signed"), "the badge: {body}");
        assert!(
            body.contains("no name an authority could"),
            "and what it means: {body}"
        );
    }

    /// Each state is one badge word, so the stream can replace it by
    /// assigning text. A state that rendered differently from the way
    /// it streams would be right until the first update.
    #[test]
    fn every_state_has_a_word_and_a_colour() {
        let states = [
            CertificateState::Local,
            CertificateState::Nameless,
            CertificateState::Working,
            CertificateState::Waiting,
            CertificateState::Trusted,
            CertificateState::Staging,
            CertificateState::Failed("refused".into()),
        ];
        let mut words = std::collections::BTreeSet::new();
        for state in &states {
            let (word, badge, dot) = state.badge();
            assert!(!word.is_empty() && !state.note().is_empty(), "{state:?}");
            assert!(
                badge.starts_with("badge") && dot.starts_with("dot"),
                "{state:?}"
            );
            words.insert(word);
        }
        assert_eq!(words.len(), states.len(), "two states read the same");
    }

    /// The reason a request is in flight has to reach the page while it
    /// is in flight — that is the whole point of the phase. Reading it
    /// from the last *error* could only ever describe attempts that had
    /// already finished badly.
    #[test]
    fn an_attempt_in_flight_outranks_what_was_stored() {
        let facts = super::super::CertificateFacts {
            issuer: "self-signed".into(),
            days_left: 0,
            trusted: false,
            source: crate::edge::certs::Source::SelfSigned,
        };
        let working = CertificateState::read(
            &facts,
            Some("node.example.com"),
            Some("refused last time".into()),
            crate::edge::acme::Phase::Working,
            false,
        );
        assert_eq!(working, CertificateState::Working);

        let settled = CertificateState::read(
            &facts,
            Some("node.example.com"),
            Some("refused last time".into()),
            crate::edge::acme::Phase::Idle,
            false,
        );
        assert_eq!(
            settled,
            CertificateState::Failed("refused last time".into())
        );

        // A node whose ACME is switched off is not "waiting" for
        // anything, and saying so would be an invitation to keep
        // looking at a page that will never change.
        let off = CertificateState::read(
            &facts,
            Some("node.example.com"),
            None,
            crate::edge::acme::Phase::Idle,
            true,
        );
        assert_eq!(off, CertificateState::Local);
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
        assert!(
            location.starts_with(&format!("{}?error=", console.node_path)),
            "{location}"
        );

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

        let response = console
            .harness
            .get(&format!("{}/live", console.node_path))
            .send()
            .await;
        response.assert_status(StatusCode::UNAUTHORIZED);
    }

    /// A node with nowhere to be called back on cannot enrol anybody,
    /// and the page has to say which of the two it is rather than
    /// offering a form whose token would fail on the far machine.
    #[tokio::test]
    async fn a_node_with_no_address_explains_why_it_cannot_enrol() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/nodes")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("no address another one could call back on"),
            "{body}"
        );
        assert!(!body.contains("/nodes/enrol"), "and offers no form: {body}");

        // And the endpoint behind it is the boundary, not the form.
        let response = console
            .harness
            .post("/nodes/enrol")
            .header("cookie", &cookie)
            .form(&[("name", "alpine")])
            .send()
            .await;
        assert!(response
            .header("location")
            .is_some_and(|to| to.contains("error=")));
    }

    /// Mint a token the way the console does, and read it back out of
    /// the one page load that will ever show it.
    async fn enrol(console: &Console, cookie: &str, name: &str) -> String {
        crate::node::settings::set_domain(&console.database, Some("hub.example"))
            .await
            .expect("domain");
        crate::network::ensure_self(&console.database, &Config::default())
            .await
            .expect("this node");

        let response = console
            .harness
            .post("/nodes/enrol")
            .header("cookie", cookie)
            .form(&[("name", name)])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").expect("redirected");

        console
            .harness
            .get(location)
            .header("cookie", cookie)
            .send()
            .await
            .body
    }

    /// The whole enrolment surface: a token, shown once, with the
    /// command that spends it — and an address held for the node it
    /// was minted for.
    #[tokio::test]
    async fn a_join_token_is_shown_once_with_what_to_do_with_it() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = enrol(&console, &cookie, "alpine").await;
        let shown = body
            .split_once("wabot-deploy join ")
            .map(|(_, rest)| rest.split('<').next().unwrap_or_default().to_string())
            .expect("the command to run, with the token in it");
        assert!(shown.starts_with("wdj1."), "{body}");
        assert!(body.contains("alpine"), "the list names it: {body}");
        assert!(body.contains("10.42.0.2"), "and holds an address: {body}");

        // A second load of the same page shows nothing: the token is
        // stored hashed, and the nonce that named it is spent. Compared
        // against the token itself rather than the prefix — the join
        // form's placeholder carries that, and matching on it would be
        // a test that passes because it found the wrong thing.
        let again = console
            .harness
            .get("/nodes")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(!again.contains(&shown), "the token came back: {again}");
        assert!(
            again.contains("Waiting"),
            "but the token is listed: {again}"
        );
    }

    /// Withdrawing gives the address back, so a token minted by mistake
    /// does not cost the overlay an address for ever.
    #[tokio::test]
    async fn a_token_can_be_withdrawn() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        enrol(&console, &cookie, "alpine").await;

        let enrolment = crate::network::enrolment::all(&console.database)
            .await
            .expect("list")
            .pop()
            .expect("one");
        console
            .harness
            .post(&format!("/nodes/enrolments/{}/withdraw", enrolment.id))
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert!(crate::network::enrolment::all(&console.database)
            .await
            .expect("list")
            .is_empty());
    }

    /// Putting a node on this one's network is not something a member
    /// of one project gets to do.
    #[tokio::test]
    async fn only_an_admin_may_enrol_a_node() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;
        crate::node::settings::set_domain(&console.database, Some("hub.example"))
            .await
            .expect("domain");
        crate::network::ensure_self(&console.database, &Config::default())
            .await
            .expect("this node");

        let response = console
            .harness
            .post("/nodes/enrol")
            .header("cookie", &member)
            .form(&[("name", "alpine")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
        assert!(
            crate::network::enrolment::all(&console.database)
                .await
                .expect("list")
                .is_empty(),
            "a member enrolled a node"
        );
    }

    /// A joined node's page is what is known about it, not this node's
    /// figures under somebody else's name. It has no memory reading and
    /// no certificate, and nothing has asked it anything.
    #[tokio::test]
    async fn a_joined_nodes_page_shows_what_is_known_and_no_more() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        joined(&console, "nd-elsewhere1").await;

        let body = console
            .harness
            .get("/nodes/nd-elsewhere1")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("10.42.0.9"), "its address: {body}");
        assert!(body.contains("on the overlay at"), "{body}");
        assert!(body.contains("Forget"), "and a way to stop listing it");
        assert!(
            !body.contains("Save domain") && !body.contains("data-cell=\"summary\""),
            "it rendered this node's own cards: {body}"
        );

        // The stream belongs to this node's page alone — it carries a
        // memory reading, and there is nowhere to get that node's.
        console
            .harness
            .get("/nodes/nd-elsewhere1/live")
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_node_can_be_forgotten_from_its_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        joined(&console, "nd-elsewhere1").await;

        console
            .harness
            .post("/nodes/forget/nd-elsewhere1")
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert_eq!(
            crate::network::find(&console.database, "nd-elsewhere1")
                .await
                .expect("query"),
            None
        );
    }

    /// The half of joining that makes it not a one-way door, from the
    /// console of the node that granted it.
    #[tokio::test]
    async fn an_authority_can_be_revoked_from_this_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        crate::network::grant(&console.database, "nd-hub0000001", "a-secret")
            .await
            .expect("grant");

        let body = console
            .harness
            .get("/nodes")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Takes instructions from"), "{body}");
        assert!(body.contains("nd-hub0000001"), "{body}");

        console
            .harness
            .post("/nodes/revoke/nd-hub0000001")
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert!(
            !crate::network::is_authorised(&console.database, "nd-hub0000001").await,
            "it may still send this node errands"
        );
    }

    /// A row about another node describes the relationship, not the
    /// machine.
    ///
    /// This shipped in v0.3.0 and was found by looking at the two test
    /// nodes side by side: the Alpine node answers to a name and has a
    /// real certificate, so its own page said `public` — correctly —
    /// while at the same moment the hub listed it as `private`. Both
    /// rows were right and the pair was nonsense. The hub does not know
    /// what that machine is; it knows it has nothing but an overlay
    /// address for it, and that is what it says now.
    #[tokio::test]
    async fn a_joined_node_is_not_described_as_something_it_may_not_be() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        joined(&console, "nd-elsewhere1").await;

        let body = console
            .harness
            .get("/nodes")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        // Scoped to that node's own card: "Add a private node" is a
        // heading on this page, and an assertion over the whole body
        // would be one that passes or fails on the wrong text.
        let card = body
            .split_once("/nodes/nd-elsewhere1")
            .map(|(_, rest)| rest.split("</a>").next().unwrap_or_default())
            .expect("the node is listed");

        assert!(card.contains("on the overlay at 10.42.0.9"), "{card}");
        assert!(
            !card.contains("private"),
            "the hub called a machine private that calls itself public: {card}"
        );
    }

    /// The node somebody is looking at is the one every other row is
    /// compared against, and alphabetical order put it second on a list
    /// of two — which is how it was noticed.
    #[tokio::test]
    async fn this_node_is_the_first_card() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        // Sorts before this node's name, which has no domain and so is
        // named after the machine.
        joined(&console, "nd-elsewhere1").await;

        let body = console
            .harness
            .get("/nodes")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        let mine = body.find(&console.node_path).expect("this node is listed");
        let theirs = body.find("/nodes/nd-elsewhere1").expect("and the other");
        assert!(mine < theirs, "this node is not the first card");
    }

    /// The reason this form exists: a node that is already installed
    /// and already answering is one somebody is looking at through a
    /// browser, and joining must not send them to find its terminal.
    #[tokio::test]
    async fn the_nodes_page_offers_to_join_a_network() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/nodes")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Join a network"), "{body}");
        assert!(body.contains("/nodes/join"), "the form posts somewhere");
        assert!(body.contains("name=\"token\""), "{body}");
    }

    /// A refusal has to land on the page with its reason, and the token
    /// must not travel back with it — not into the query string, not
    /// into the form. It is the one secret this page handles.
    #[tokio::test]
    async fn a_join_that_is_refused_says_why_without_echoing_the_token() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .post("/nodes/join")
            .header("cookie", &cookie)
            .form(&[("token", "wdj1.not-a-real-token")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").unwrap_or_default();
        assert!(location.starts_with("/nodes?error="), "{location}");
        assert!(
            !location.contains("wdj1"),
            "the token came back in the URL: {location}"
        );

        // And nothing was granted on the strength of it.
        assert!(crate::network::authorities(&console.database)
            .await
            .expect("query")
            .is_empty());

        let body = console
            .harness
            .get(location)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("damaged"),
            "the reason reaches the page: {body}"
        );
        assert!(!body.contains("not-a-real-token"), "{body}");
    }

    /// Granting another node authority over this one is the largest
    /// thing anybody can do from this console.
    #[tokio::test]
    async fn only_an_admin_may_join_a_network() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        let response = console
            .harness
            .post("/nodes/join")
            .header("cookie", &member)
            .form(&[("token", "wdj1.whatever")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
    }

    /// The console's half of phase 3: asking another node to run a
    /// service this one has, and seeing what became of the asking.
    #[tokio::test]
    async fn a_service_can_be_sent_to_a_joined_node() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        joined(&console, "nd-elsewhere1").await;

        let project = crate::platform::projects::create(&console.database, "shared")
            .await
            .expect("project");
        let service = crate::platform::services::create(
            &console.database,
            &project.id,
            "web",
            "hub.example.com/shared/web@sha256:abc",
            &[],
        )
        .await
        .expect("service");

        console
            .harness
            .post("/nodes/nd-elsewhere1/host")
            .header("cookie", &cookie)
            .form(&[("service", service.id.as_str())])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let queued = crate::network::errand::all(&console.database)
            .await
            .expect("errands");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].node_id, "nd-elsewhere1");
        assert!(!queued[0].done(), "nothing has collected it yet");

        // The far node pulls the same bytes this one runs, with a
        // credential minted for the errand — a tag it resolved itself
        // would be two nodes disagreeing about one service.
        let waiting = crate::network::errand::waiting(&console.database, "nd-elsewhere1")
            .await
            .expect("waiting");
        let host: crate::network::errand::Host =
            serde_json::from_value(waiting[0].payload.clone()).expect("a host errand");
        assert_eq!(host.image, "hub.example.com/shared/web@sha256:abc");
        assert_eq!(host.registry, "hub.example.com");
        assert!(!host.secret.is_empty(), "nothing to pull it with");

        // And the page says what became of it.
        let body = console
            .harness
            .get("/nodes/nd-elsewhere1")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Collected"), "{body}");
    }

    /// An image from somebody else's registry is one the far node can
    /// pull without anything from here — and sending it a credential
    /// for *this* registry would be sending it somewhere it does not
    /// belong.
    #[tokio::test]
    async fn a_service_whose_image_names_no_registry_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        joined(&console, "nd-elsewhere1").await;

        let project = crate::platform::projects::create(&console.database, "shared")
            .await
            .expect("project");
        let service = crate::platform::services::create(
            &console.database,
            &project.id,
            "web",
            "alpine:3.23",
            &[],
        )
        .await
        .expect("service");

        let response = console
            .harness
            .post("/nodes/nd-elsewhere1/host")
            .header("cookie", &cookie)
            .form(&[("service", service.id.as_str())])
            .send()
            .await;

        let location = response.header("location").unwrap_or_default();
        assert!(location.contains("error="), "{location}");
        assert!(crate::network::errand::all(&console.database)
            .await
            .expect("errands")
            .is_empty());
    }

    /// A node that arrived through a join, as the API writes it.
    async fn joined(console: &Console, id: &str) {
        crate::network::save(
            &console.database,
            &crate::network::Node {
                id: id.into(),
                name: "alpine.example".into(),
                kind: Kind::Private,
                endpoint: None,
                public_key: Some("0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: Some(super::super::now_ms()),
            },
        )
        .await
        .expect("save");
    }

    #[tokio::test]
    async fn the_nodes_pages_need_a_session() {
        let console = Console::new().await;
        console.signed_in().await;

        for path in ["/nodes", &console.node_path] {
            let response = console.harness.get(path).send().await;
            assert_eq!(
                response.header("location"),
                Some("/sign-in"),
                "{path} was served to nobody"
            );
        }
    }
}
