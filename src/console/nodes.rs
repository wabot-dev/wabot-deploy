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
                        <p class="slug-preview">("wabot-deploy ")(node.version)</p>
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
        let here = "/nodes/local";

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
        let here = "/nodes/local";

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
        let known = crate::node::settings::domain(&self.state.database, &self.state.config).await;
        if crate::node::find(known, &id).is_none() {
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
            .get("/nodes/local")
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
            .navigate("/nodes/local")
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
            .get("/nodes/local")
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
            .get("/nodes/local")
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

        let response = console.harness.get("/nodes/local/live").send().await;
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
