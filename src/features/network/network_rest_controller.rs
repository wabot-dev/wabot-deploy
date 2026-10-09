//! What the network's pages post to, and the stream they feed.
//!
//! The live cells live here because the stream is what sends them: the
//! pages paint from the same numbers once, through the components, and
//! nothing else reads these.

use std::collections::BTreeMap;
use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::infra::network::{self};
use crate::infra::node::memory::{self, Snapshot};

use crate::web::console::auth::{signed_in, SessionMiddleware};
use crate::web::console::ConsoleState;
use crate::web::views::components::node_certificate::{CertificateCells, CertificateState};

#[injectable]
pub struct NetworkRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl NetworkRestController {
    /// Mint a join token for a node that does not exist here yet.
    ///
    /// Everything the other machine needs is decided here, because
    /// everything it needs is this node's to decide: the address on
    /// this node's overlay, the key it will talk to, and the secret it
    /// will authenticate with. The token is shown once and then only
    /// its hash is kept.
    #[post("/network/enrol")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn enrol(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        // Enrolling a node is not a project-level decision: it grants
        // whoever holds the token a place on this node's network.
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }
        let here = "/network/invite";

        let form = match crate::web::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        // Optional, and it is **not** the node's name: a node's name is
        // its own, arrives on the join callback, and is what the list of
        // nodes shows. This is a note on a token that has not been spent
        // yet, for the operator who minted three of them this afternoon
        // — so it is bounded, and it is not required.
        let asked = crate::features::network::mint_join_token_request::MintJoinTokenRequest {
            name: crate::web::console::auth::field(&form, "name").to_string(),
            requires: crate::web::console::auth::capabilities_ticked(&form, "require-"),
            offers: crate::web::console::auth::capabilities_ticked(&form, "offer-"),
        };
        let token = match crate::features::network::node_user_mint_join_token::run(
            &self.state.database,
            &self.state.config,
            &account,
            &asked,
        )
        .await
        {
            Ok(token) => token,
            Err(reason) => return Ok(crate::web::console::auth::back_with_error(here, &reason)),
        };

        // Through the reveal store, not the query string: the token is
        // the secret, and a query string is read by the address bar, the
        // history and every refresh.
        let nonce = self.state.reveals.stash(token);
        Ok(crate::web::console::auth::see_other(&format!(
            "/network/invite/shown?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("nonce", &nonce)
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
    #[post("/network/join")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn join(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        // Granting another node authority over this one is the largest
        // thing anybody can do from this console.
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }
        let here = "/network/join";

        let form = match crate::web::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        use crate::features::network::join_token_spent_response::JoinTokenSpentResponse;
        let asked = crate::features::network::spend_join_token_request::SpendJoinTokenRequest {
            token: crate::web::console::auth::field(&form, "token").to_string(),
            agreed: form.contains_key("agreed"),
            // The form's spelling stops here. A checkbox arrives only
            // when ticked, so presence is the answer.
            accepted: crate::domain::values::capability::Capability::ALL
                .into_iter()
                .filter(|capability| form.contains_key(&format!("accept-{}", capability.name())))
                .collect(),
        };
        match crate::features::network::node_user_spend_join_token::run(
            &self.state.database,
            &self.state.config,
            &account,
            &asked,
        )
        .await
        {
            Ok(JoinTokenSpentResponse::Review(token)) => {
                let nonce = self.state.reveals.stash(token);
                Ok(crate::web::console::auth::see_other(&format!(
                    "{here}?{}",
                    form_urlencoded::Serializer::new(String::new())
                        .append_pair("review", &nonce)
                        .finish()
                )))
            }
            Ok(JoinTokenSpentResponse::Joined(authority)) => {
                Ok(crate::web::console::auth::see_other(&format!(
                    "{here}?{}",
                    form_urlencoded::Serializer::new(String::new())
                        .append_pair("joined", &authority)
                        .finish()
                )))
            }
            Err(reason) => Ok(crate::web::console::auth::back_with_error(here, &reason)),
        }
    }

    /// Withdraw a token nobody has spent.
    #[post("/network/enrolments/:enrolment/withdraw")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn withdraw(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        if let Some(id) = crate::web::console::auth::segments(&path).get(2) {
            if let Err(reason) = crate::features::network::node_user_withdraw_enrolment::run(
                &self.state.database,
                &account,
                id,
            )
            .await
            {
                return Ok(crate::web::console::auth::back_with_error(
                    "/network/invite",
                    &reason,
                ));
            }
        }
        Ok(crate::web::console::auth::see_other("/network/invite"))
    }

    /// Take a node out of service, or put it back.
    ///
    /// **The mark and the moving are one press**, because a mark without
    /// the moving is a node quietly holding work nobody will ever choose
    /// it for again, and moving without the mark is a drain the planner
    /// undoes on its next decision.
    ///
    /// What comes back is a count of rows moved *here* and a sentence
    /// per copy that could not be — never "drained", because the far
    /// node stops when it next collects and this console cannot know
    /// that it did.
    #[post("/network/:node/drain")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn drain_node(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        let Some(id) = crate::web::console::auth::segments(&path).get(1).cloned() else {
            return Ok(crate::web::console::auth::see_other("/network"));
        };
        let form = match crate::web::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let here = format!("/network/{id}");

        use crate::features::network::node_drained_response::NodeDrainedResponse;
        match crate::features::network::node_user_drain_node::run(
            &self.state.database,
            &self.state.deployer,
            &account,
            id,
            crate::web::console::auth::field(&form, "draining") == "on",
        )
        .await
        {
            Ok(NodeDrainedResponse::BackInService) => {
                Ok(crate::web::console::auth::see_other(&here))
            }
            Ok(NodeDrainedResponse::Moved { moved, refused }) if !refused.is_empty() => {
                Ok(crate::web::console::auth::back_with_error(
                    &here,
                    &crate::web::views::texts::sentences::copies_moved(moved, &refused),
                ))
            }
            Ok(NodeDrainedResponse::Moved { .. }) => {
                Ok(crate::web::console::auth::see_other(&here))
            }
            Err(reason) => Ok(crate::web::console::auth::back_with_error(&here, &reason)),
        }
    }

    #[post("/network/forget/:node")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn forget_node(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        if let Some(id) = crate::web::console::auth::segments(&path).get(2) {
            if let Err(reason) = crate::features::network::node_user_forget_node::run(
                &self.state.database,
                &account,
                id,
            )
            .await
            {
                return Ok(crate::web::console::auth::back_with_error(
                    "/network", &reason,
                ));
            }
        }
        Ok(crate::web::console::auth::see_other("/network"))
    }

    /// Stop taking instructions from a node.
    ///
    /// The half of joining that makes it not a one-way door. It takes
    /// effect here and nowhere else — the other node keeps whatever it
    /// recorded, and finds out by being refused.
    #[post("/network/revoke/:node")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revoke_authority(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }

        let path = request.uri().path().to_string();
        if let Some(id) = crate::web::console::auth::segments(&path).get(2) {
            if let Err(reason) = crate::features::network::node_user_revoke_authority::run(
                &self.state.database,
                &account,
                id,
            )
            .await
            {
                return Ok(crate::web::console::auth::back_with_error(
                    "/network", &reason,
                ));
            }
        }
        Ok(crate::web::console::auth::see_other("/network"))
    }

    /// What this node agrees to do for one other node.
    ///
    /// The whole set each time, ticked and unticked together: a form that
    /// could only add would make "I have changed my mind about that one"
    /// impossible to say. Withdrawing needs a row of its own —
    /// `capability::set_granted` writes it — because the boot pass that
    /// re-reads what an enrolment offered would otherwise put it back.
    #[post("/network/:node/grants")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_grants(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = crate::web::console::auth::segments(&path)
            .get(1)
            .map(|id| id.to_string())
        else {
            return Ok(crate::web::console::auth::see_other("/network"));
        };
        let form = match crate::web::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let here = format!("/network/{id}");
        let asked = crate::features::network::set_grants_request::SetGrantsRequest {
            node: id.to_string(),
            agreed: crate::web::console::auth::capabilities_ticked(&form, ""),
        };
        match crate::features::network::node_user_set_grants::run(
            &self.state.database,
            &account,
            &asked,
        )
        .await
        {
            Ok(()) => Ok(crate::web::console::auth::see_other(&here)),
            Err(reason) => Ok(crate::web::console::auth::back_with_error(&here, &reason)),
        }
    }

    /// Ask a node to be running a release.
    ///
    /// Two writes and neither is the instruction: the decision goes on
    /// that node's row, and `dispatch_versions` turns it into an errand —
    /// here and at every boot. A form that queued directly would be an
    /// instruction that exists only if the button was pressed while both
    /// machines were up.
    #[post("/network/:node/version")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_wanted_version(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = crate::web::console::auth::segments(&path)
            .get(1)
            .map(|id| id.to_string())
        else {
            return Ok(crate::web::console::auth::see_other("/network"));
        };
        let form = match crate::web::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let here = format!("/network/{id}");
        match crate::features::network::node_user_set_wanted_version::run(
            &self.state.database,
            &account,
            &id,
            crate::web::console::auth::field(&form, "version"),
        )
        .await
        {
            Ok(()) => Ok(crate::web::console::auth::see_other(&here)),
            Err(reason) => Ok(crate::web::console::auth::back_with_error(&here, &reason)),
        }
    }

    /// Ask a node to do something once.
    ///
    /// No deduplication and no desired state: pressing it twice asks
    /// twice, and each asking is its own errand with its own id. What that
    /// id buys is that the *same* asking cannot be obeyed twice, which
    /// matters because the thing it asks for kills the process that would
    /// have reported it done.
    #[post("/network/:node/command")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn ask_node_once(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = crate::web::console::auth::segments(&path)
            .get(1)
            .map(|id| id.to_string())
        else {
            return Ok(crate::web::console::auth::see_other("/network"));
        };
        let form = match crate::web::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let here = format!("/network/{id}");
        let action = match crate::web::console::auth::field(&form, "action") {
            "restart" => network::errand::Action::Restart,
            "back-up" => network::errand::Action::BackUp,
            // Not `Action::Unknown`: that is what a *newer node's* word
            // parses to on the way in, and sending it would be this
            // console asking for something it has no name for.
            other => {
                tracing::warn!(action = %other, "no such thing to ask a node for");
                return Ok(crate::web::console::auth::back_with_error(
                    &here,
                    "no such action",
                ));
            }
        };
        match crate::features::network::node_user_ask_node_once::run(
            &self.state.database,
            &account,
            &id,
            action,
        )
        .await
        {
            Ok(()) => Ok(crate::web::console::auth::see_other(&here)),
            Err(reason) => Ok(crate::web::console::auth::back_with_error(&here, &reason)),
        }
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
    #[get("/network/:node/live")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn live_stream(&self, request: Request) -> RestResult<Response> {
        if !signed_in(&self.auth)
            .is_some_and(|account| crate::domain::authz::node_user::manages_the_machine(&account))
        {
            // A stream is not a page, so this is a status rather than a
            // redirect: an EventSource cannot follow one usefully.
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        }

        let path = request.uri().path().to_string();
        let id = crate::web::console::auth::segments(&path)
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
        // A joined node's page has a stream too now, and carries only
        // what this node can honestly say about it: the errands it was
        // sent, and how far each has got. Memory and certificates stay
        // behind `mine` — those are answers only that machine can give,
        // and streaming this one's figures under its name would be the
        // page inventing them.
        if !mine
            && network::find(&self.state.database, &id)
                .await
                .unwrap_or_default()
                .is_none()
        {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        }

        let state = self.state.clone();
        let mut phase = state.certificates.watch();
        let stream = async_stream::stream! {
            // The previous CPU reading, which is the whole of why this
            // figure can exist here and nowhere else: a percentage is the
            // difference between two samples, and a stream is the one
            // place that naturally has both. The first tick carries none —
            // there is nothing to subtract from — and the page shows a
            // dash rather than a number that would mean "since boot".
            let mut before: Option<crate::infra::node::cpu::Sample> = None;
            loop {
                let snapshot = state.deployer.memory().await;
                let now = crate::infra::node::cpu::sample(&state.deployer.cgroups().await);
                let busy = before
                    .as_ref()
                    .and_then(|before| crate::infra::node::cpu::between(before, &now));
                before = Some(now);
                let facts = crate::web::console::certificate_facts(&state).await;
                let domain =
                    crate::infra::node::settings::domain(&state.database, &state.config).await;
                let certificate = CertificateState::read(
                    &facts,
                    domain.as_deref(),
                    crate::infra::edge::policy::for_name(
                        &state.database,
                        &state.config,
                        domain.as_deref().unwrap_or(crate::infra::edge::certs::FALLBACK_NAME),
                    )
                    .await
                    .last_error
                    .or(crate::infra::node::settings::acme_error(&state.database).await),
                    state.certificates.phase(),
                    state.config.acme.disabled,
                );
                let payload = serde_json::to_string(&Live {
                    inner: cells(&snapshot, busy.as_ref()),
                    certificate: crate::web::views::components::node_certificate::certificate_cells(&certificate, &facts, domain.as_deref()),
                    errands: errand_cells(&state.database, &id).await,
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
    /// One per errand this node sent that one, keyed by id.
    ///
    /// Empty on this node's own page — it sends itself none — and the
    /// only thing on a joined node's, where the memory and certificate
    /// figures would be this machine's own under somebody else's name.
    errands: BTreeMap<String, ErrandCell>,
}

/// One errand row, as its badge reads.
#[derive(serde::Serialize)]
struct ErrandCell {
    badge: &'static str,
    dot: &'static str,
    word: &'static str,
    /// The reason, when it was refused. Empty otherwise.
    failure: String,
}

/// How far each errand sent to `node_id` has got.
///
/// A node collects on a fifteen-second timer and reports when it is
/// done, so "waiting to be collected" is precisely the state somebody
/// stares at after pressing the button — and it used to stay on screen
/// long after the far node had finished.
async fn errand_cells(
    database: &wabot::sqlite::SqliteDatabase,
    node_id: &str,
) -> BTreeMap<String, ErrandCell> {
    let mut cells = BTreeMap::new();
    let Ok(orders) = network::errand::all(database).await else {
        return cells;
    };

    for order in orders
        .into_iter()
        .filter(|record| record.node_id == node_id)
    {
        let cell = if let Some(reason) = &order.error {
            ErrandCell {
                badge: "badge badge-danger",
                dot: "dot dot-danger",
                word: "Refused",
                failure: reason.clone(),
            }
        } else if order.done() {
            ErrandCell {
                badge: "badge badge-success",
                dot: "dot dot-success",
                word: "Done",
                failure: String::new(),
            }
        } else if order.taken_at.is_some() {
            ErrandCell {
                badge: "badge badge-info",
                dot: "dot dot-info dot-pulse",
                word: "Collected",
                failure: String::new(),
            }
        } else {
            ErrandCell {
                badge: "badge badge-info",
                dot: "dot dot-info",
                word: "Waiting to be collected",
                failure: String::new(),
            }
        };
        cells.insert(order.id.clone(), cell);
    }
    cells
}

/// What the script writes where.
///
/// Built here rather than in the browser so the two halves cannot
/// disagree about formatting: the same `human()` renders the first
/// paint and every update after it.
#[derive(serde::Serialize)]
pub(crate) struct Cells {
    pub cells: BTreeMap<String, String>,
    pub bars: BTreeMap<String, String>,
}

fn cells(snapshot: &Snapshot, busy: Option<&crate::infra::node::cpu::Busy>) -> Cells {
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
        // A dash until there are two readings to subtract. Not "0 %": a
        // machine at nought is a machine somebody would believe.
        // Millicores, against what the machine has: a thousand is one
        // core, and the capacity is what makes the first number mean
        // something without knowing the box.
        (
            "cpu".to_string(),
            busy.map(|busy| format!("{}m of {}m", busy.node, busy.capacity))
                .unwrap_or_else(|| "—".into()),
        ),
    ]);

    let bars = BTreeMap::from([
        (
            "node".to_string(),
            crate::features::network::node_usage_component::percent(snapshot, snapshot.node),
        ),
        (
            "runtime".to_string(),
            crate::features::network::node_usage_component::percent(snapshot, runtime),
        ),
        (
            "containers".to_string(),
            crate::features::network::node_usage_component::percent(snapshot, containers),
        ),
        (
            "rest".to_string(),
            crate::features::network::node_usage_component::percent(snapshot, snapshot.rest()),
        ),
    ]);

    Cells { cells, bars }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::Config;
    use crate::infra::network::Kind;
    use crate::web::console::tests::Console;
    use hypertext::Renderable;
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
        let rendered = crate::features::network::node_usage_component::memory_card(
            &snapshot,
            &Default::default(),
            (None, None),
        )
        .render()
        .into_inner();
        let payload = cells(&snapshot, None);

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

    /// A bar the browser will actually take.
    ///
    /// The stream assigns these to `style.width`, which refuses anything
    /// it cannot parse and refuses it silently — so `width:12.34%`, right
    /// as an attribute, was a meter that never moved after the first
    /// paint while the numbers beside it changed every second. The
    /// attribute and the assignment want different strings, and only one
    /// of them says so out loud.
    #[test]
    fn a_streamed_bar_is_a_value_and_the_attribute_is_a_declaration() {
        let snapshot = snapshot();

        for (key, value) in &cells(&snapshot, None).bars {
            assert!(
                !value.contains("width"),
                "{key} carries a declaration where a value belongs: {value}"
            );
            assert!(value.ends_with('%'), "{key} is not a percentage: {value}");
        }

        // And the first paint still needs the whole declaration, because
        // there it is the attribute.
        assert!(crate::features::network::node_usage_component::memory_card(
            &snapshot,
            &Default::default(),
            (None, None)
        )
        .render()
        .into_inner()
        .contains("style=\"width:"));
    }

    /// Every meter's width goes through `bar_width`.
    ///
    /// The test above asserts the shape for the *memory* card, and the
    /// disk card shipped with its own closure formatting a bare
    /// `7.0000%` — beside a correct helper of the same name in the same
    /// file. A `style` attribute wants a declaration, so the browser
    /// discarded it and the bar rendered flat while the figures under it
    /// were right: it read as a node using no disk rather than as a broken
    /// bar. Reported by Jorge.
    ///
    /// So this reads the source instead of one card's output. One place to
    /// be right is worth nothing unless everything goes through it, and a
    /// third card is exactly how this happened a second time.
    ///
    /// **It read nothing for months.** It scanned the controllers, and the
    /// meters had moved into the views when the controllers stopped
    /// drawing: no file it read had a `style` attribute in it, and with no
    /// floor that passed. The organisation's quota card wrote its own
    /// width the whole time, and was found by a move, not by this.
    #[test]
    fn every_meter_width_is_a_declaration() {
        // Assembled rather than written out, or this scan finds its own
        // predicate and reports the line it is reading from.
        let needle = concat!("style", "=(");
        let mut written = 0;
        let mut wrong = Vec::new();
        for path in crate::tree::rust_files(&["src/web/console", "src/web/views", "src/features"]) {
            // The helpers' own file calls them by their short names.
            let home = path
                .file_stem()
                .is_some_and(|stem| stem == "node_usage_component");
            let source = std::fs::read_to_string(&path).expect("read");
            for (number, line) in source.lines().enumerate() {
                if line.trim_start().starts_with("//") || !line.contains(needle) {
                    continue;
                }
                written += 1;
                let through = ["width(", "bar_width("].iter().any(|call| {
                    line.contains(&format!("node_usage_component::{call}"))
                        || (home && line.contains(&format!("({call}")))
                });
                if !through {
                    wrong.push(format!("{}:{}", path.display(), number + 1));
                }
            }
        }
        assert!(
            written > 5,
            "the scan found {written} style attributes, so it is reading the wrong place"
        );
        assert!(
            wrong.is_empty(),
            "these write a style attribute without going through bar_width: {wrong:#?}"
        );
    }

    /// And what it produces is a declaration, in both directions of the
    /// confusion that caused this twice.
    #[test]
    fn a_meter_width_is_a_declaration_and_not_a_bare_value() {
        assert_eq!(
            crate::features::network::node_usage_component::bar_width(7.5),
            "width:7.5000%"
        );
        // Zero is still sized, rather than left for whatever was there.
        assert_eq!(
            crate::features::network::node_usage_component::bar_width(0.0),
            "width:0.0000%"
        );
    }

    /// The figures are formatted once, on the server, so the first
    /// paint and every update after it read the same.
    #[test]
    fn the_first_paint_and_the_stream_agree() {
        let snapshot = snapshot();
        let rendered = crate::features::network::node_usage_component::memory_card(
            &snapshot,
            &Default::default(),
            (None, None),
        )
        .render()
        .into_inner();

        for value in cells(&snapshot, None).cells.values() {
            if value.contains("of 0 B") {
                continue; // swap, which this machine does not have
            }
            assert!(
                rendered.contains(value.as_str()),
                "the page does not show {value}"
            );
        }
    }

    /// The network is where you look; `/node` is what you change.
    ///
    /// They were one page called `/nodes`, so "how are my nodes" and
    /// "what does this machine do" had the same URL and the same word in
    /// the nav — and adding a node meant minting a token on one page and
    /// watching for it on the same one. Asked for by Jorge, who was not
    /// sure of the shape; this is the shape.
    ///
    /// The two old paths still answer, because a bookmark is somebody's
    /// and the plural named the list.
    #[tokio::test]
    async fn looking_at_the_network_and_configuring_this_node_are_two_places() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! get {
            ($path:expr) => {
                console
                    .harness
                    .get($path)
                    .header("cookie", &cookie)
                    .send()
                    .await
            };
        }

        // The old paths are redirects, and each goes where its name did.
        assert_eq!(
            get!("/nodes").header("location"),
            Some("/network"),
            "the plural named the list"
        );
        assert_eq!(
            get!("/settings").header("location"),
            Some("/node"),
            "and the gear lands on this machine"
        );

        let network = get!("/network").body;
        // The list, and the two things that change what is on it — in the
        // side nav, which is where this area's own column was empty. They
        // were buttons in the page's header first, and an empty sidebar
        // beside them was the thing Jorge saw.
        let sidebar = network
            .split_once(r#"<div class="side-inner">"#)
            .map(|(_, rest)| rest.split("</aside>").next().unwrap_or_default())
            .expect("a sidebar");
        assert!(
            sidebar.contains("/network/invite") && sidebar.contains("/network/join"),
            "the network area's column is empty: {sidebar}"
        );
        // And none of the switches: those are settings.
        assert!(
            !network.contains("/node/capabilities"),
            "the capability switch is on the list: {network}"
        );
        assert!(
            !network.contains("/node/certificate"),
            "and so is the certificate: {network}"
        );

        // `/node` is a redirect to its first tab, and the two forms are on
        // two of them since Jorge asked for tabs here as well.
        assert_eq!(get!("/node").header("location"), Some("/node/domain"));
        let certificate = get!("/node/domain").body;
        assert!(certificate.contains("/node/certificate"), "{certificate}");
        let allows = get!("/node/allows").body;
        assert!(allows.contains("/node/capabilities"), "{allows}");
        // Nothing that adds a node: these pages are about the one machine.
        for body in [&certificate, &allows] {
            assert!(
                !body.contains("/network/enrol"),
                "the invite form is in settings: {body}"
            );
        }

        // Each form is on the page it comes back to. A save that lands
        // somewhere else is a save that hides what it did.
        //
        // The invite *form* is asserted by
        // `a_node_with_no_address_explains_why_it_cannot_enrol` and not
        // here, because this node has neither an endpoint nor a domain —
        // so `enrol_card` deliberately renders the refusal instead. What
        // this pins is that the page is the card's home.
        assert!(get!("/network/invite").body.contains("Invite a node"));
        assert!(get!("/network/join").body.contains("name=\"token\""));
    }

    #[tokio::test]
    async fn the_list_shows_this_node() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/network")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Network"), "{body}");
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
            .get("/network/somewhere-else")
            .header("cookie", &cookie)
            .send()
            .await;
        response.assert_status(StatusCode::FOUND);
        assert_eq!(response.header("location"), Some("/network"));
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
            .get("/network/invite")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("no address another one could call back on"),
            "{body}"
        );
        assert!(
            !body.contains("/network/enrol"),
            "and offers no form: {body}"
        );

        // And the endpoint behind it is the boundary, not the form.
        let response = console
            .harness
            .post("/network/enrol")
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
        crate::infra::node::settings::set_domain(&console.database, Some("hub.example"))
            .await
            .expect("domain");
        crate::infra::network::ensure_self(&console.database, &Config::default())
            .await
            .expect("this node");

        let response = console
            .harness
            .post("/network/enrol")
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

    /// The whole enrolment surface: a token, shown once, on a page whose
    /// arrow goes back to the one that minted it.
    ///
    /// **The token alone, and not the command.** It is pasted into the
    /// other node's console far more often than into its terminal, and a
    /// prefix somebody has to delete first is worse than one they can
    /// type — reported by Jorge. So what is asserted is that the block
    /// holds the token and nothing before it.
    #[tokio::test]
    async fn a_join_token_is_shown_once_with_what_to_do_with_it() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = enrol(&console, &cookie, "alpine").await;
        let shown = body
            .split_once("data-copied-label=")
            .and_then(|(_, rest)| rest.split_once('>'))
            .and_then(|(_, rest)| rest.split_once("</pre>"))
            .map(|(secret, _)| secret.trim().to_string())
            .expect("the block that holds it");
        assert!(shown.starts_with("wdj1."), "{body}");
        assert!(
            !shown.contains(' '),
            "the token came with something in front of it: {shown:?}"
        );
        // **The name and the address are not here.** The token has a
        // page of its own now, with one thing on it; the list of what
        // has been minted stays on the page that minted them, which is
        // where the arrow goes back to.
        assert!(
            body.contains("/network/invite"),
            "no way back to the page that minted it: {body}"
        );

        // A second load of the same page shows nothing: the token is
        // stored hashed, and the nonce that named it is spent. Compared
        // against the token itself rather than the prefix — the join
        // form's placeholder carries that, and matching on it would be
        // a test that passes because it found the wrong thing.
        let again = console
            .harness
            .get("/network/invite")
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

    /// **A node's name is its own.** It says what it is on the join
    /// callback, and that is the name the nodes list shows — so the
    /// mint form has nothing to ask for. It used to require one, which
    /// gave one machine two names differing by which table you read:
    /// the same fault the databases had, one floor up.
    ///
    /// What is left is a note for the operator with three unspent
    /// tokens, and a note is not something to be stopped by.
    #[tokio::test]
    async fn a_token_can_be_minted_without_naming_the_node() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = enrol(&console, &cookie, "").await;
        assert!(
            body.contains("wabot-deploy join "),
            "an unnamed token was refused: {body}"
        );

        let listed = console
            .harness
            .get("/network/invite")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(listed.contains("Waiting"), "{listed}");
        // The address is what identifies an unspent token, and it is
        // already in the row beside the empty note.
        assert!(listed.contains("10.42.0.2"), "{listed}");
    }

    /// Withdrawing gives the address back, so a token minted by mistake
    /// does not cost the overlay an address for ever.
    #[tokio::test]
    async fn a_token_can_be_withdrawn() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        enrol(&console, &cookie, "alpine").await;

        let enrolment = crate::infra::network::enrolment::all(&console.database)
            .await
            .expect("list")
            .pop()
            .expect("one");
        console
            .harness
            .post(&format!("/network/enrolments/{}/withdraw", enrolment.id))
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert!(crate::infra::network::enrolment::all(&console.database)
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
        crate::infra::node::settings::set_domain(&console.database, Some("hub.example"))
            .await
            .expect("domain");
        crate::infra::network::ensure_self(&console.database, &Config::default())
            .await
            .expect("this node");

        let response = console
            .harness
            .post("/network/enrol")
            .header("cookie", &member)
            .form(&[("name", "alpine")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
        assert!(
            crate::infra::network::enrolment::all(&console.database)
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
            .get("/network/nd-elsewhere1")
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

        // It has a stream now, and only the errands are in it: a node
        // collects on a fifteen-second timer, so "waiting to be
        // collected" is exactly the badge somebody watches after
        // pressing the button. Memory and certificates stay behind
        // `mine` — those are answers only that machine can give, and
        // streaming this one's under its name would be invention.
        //
        // Asserted from the page rather than by calling `/live`: a
        // successful stream never ends, and a test that asks for one
        // waits for a body that will not come. That is what this test
        // did for an hour when the endpoint stopped answering 404 —
        // see `errands_are_reported_for_the_node_they_were_sent_to`
        // for the contents, which is the part worth pinning anyway.
        assert!(
            body.contains("data-island=\"node-live\""),
            "a joined node's page follows its errands: {body}"
        );
    }

    /// What the joined node's page shows about each instruction, and
    /// the reason it is tested here rather than through the stream: a
    /// stream that works never ends, so asking for one in a test is
    /// asking for a body that never arrives.
    #[tokio::test]
    async fn errands_are_reported_for_the_node_they_were_sent_to() {
        let console = Console::new().await;
        joined(&console, "nd-elsewhere1").await;

        network::errand::queue(
            &console.database,
            "nd-elsewhere1",
            network::errand::Kind::Host,
            &serde_json::json!({ "project": "shared" }),
        )
        .await
        .expect("queued");

        let cells = errand_cells(&console.database, "nd-elsewhere1").await;
        let (_, cell) = cells.iter().next().expect("one errand");
        assert_eq!(cell.word, "Waiting to be collected");

        assert!(
            errand_cells(&console.database, "nd-somebody-else")
                .await
                .is_empty(),
            "an errand belongs to the node it was sent to"
        );
    }

    #[tokio::test]
    async fn a_node_can_be_forgotten_from_its_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        joined(&console, "nd-elsewhere1").await;

        console
            .harness
            .post("/network/forget/nd-elsewhere1")
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert_eq!(
            crate::infra::network::find(&console.database, "nd-elsewhere1")
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
        crate::infra::network::grant(&console.database, "nd-hub0000001", "a-secret")
            .await
            .expect("grant");

        let body = console
            .harness
            .get("/network")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Takes instructions from"), "{body}");
        assert!(body.contains("nd-hub0000001"), "{body}");

        console
            .harness
            .post("/network/revoke/nd-hub0000001")
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert!(
            !crate::infra::network::is_authorised(&console.database, "nd-hub0000001").await,
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
            .get("/network")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        // Scoped to that node's own card: "Invite a node" is a
        // heading on this page, and an assertion over the whole body
        // would be one that passes or fails on the wrong text.
        let card = body
            .split_once("/network/nd-elsewhere1")
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
            .get("/network")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        let mine = body.find(&console.node_path).expect("this node is listed");
        let theirs = body.find("/network/nd-elsewhere1").expect("and the other");
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
            .get("/network/join")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Join a network"), "{body}");
        assert!(body.contains("/network/join"), "the form posts somewhere");
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
            .post("/network/join")
            .header("cookie", &cookie)
            .form(&[("token", "wdj1.not-a-real-token")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").unwrap_or_default();
        assert!(location.starts_with("/network/join?error="), "{location}");
        assert!(
            !location.contains("wdj1"),
            "the token came back in the URL: {location}"
        );

        // And nothing was granted on the strength of it.
        assert!(crate::infra::network::authorities(&console.database)
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
            .post("/network/join")
            .header("cookie", &member)
            .form(&[("token", "wdj1.whatever")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
    }

    /// A node that arrived through a join, as the API writes it.
    async fn joined(console: &Console, id: &str) {
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: id.into(),
                name: "alpine.example".into(),
                kind: Kind::Private,
                endpoint: None,
                public_key: Some("0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: Some(crate::web::console::now_ms()),
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("save");
    }

    #[tokio::test]
    async fn the_nodes_pages_need_a_session() {
        let console = Console::new().await;
        console.signed_in().await;

        for path in [
            "/network",
            "/network/invite",
            "/network/join",
            "/node",
            &console.node_path,
        ] {
            let response = console.harness.get(path).send().await;
            assert_eq!(
                response.header("location"),
                Some("/sign-in"),
                "{path} was served to nobody"
            );
        }
    }
}
