//! Where a joining node says who it is.
//!
//! One endpoint, and it is the only place a node writes a row on
//! somebody else's behalf. What makes that safe is not the shape of the
//! request: it is that the caller is holding a secret this node minted,
//! for one enrolment, that expires and can only be spent once.
//!
//! ## Why the joining node brings its own id
//!
//! The alternative is allocating one here and putting it in the token.
//! It is tempting — this node already allocates the overlay address —
//! and it is wrong: a node that joins two authorities would then be two
//! nodes with two identities, and every question about "which node ran
//! this" would have two answers. So identity is the node's own, minted
//! when it was installed, and what this node allocates is only the
//! address on *its* overlay.
//!
//! The cost is that a stranger's id could collide with one already
//! known here. Ids are twelve random characters, so that is not an
//! accident — it is a claim, and it is refused rather than merged. The
//! upsert underneath would otherwise let a joining node overwrite the
//! row it collided with, including this node's own.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::axum::Router;
use wabot::rest::RestResult;
use wabot::sqlite::SqliteDatabase;

use super::{enrolment, errand, Kind, Node};
use crate::platform::now_ms;

/// What a node says about itself when it arrives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arriving {
    /// Its own id, minted when it was installed.
    pub node: String,
    pub name: String,
    /// Its overlay public key, so the authority has everything it needs
    /// to build a tunnel when there is one to build.
    pub public_key: String,
}

/// What the authority answers with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accepted {
    /// The authority's own id, which is what the joining node should
    /// have recorded — returned so a mismatch is visible rather than
    /// discovered in phase 3.
    pub authority: String,
    pub name: String,
    pub overlay_ip: String,
}

/// How an errand went, as the node that carried it out reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    /// `None` — or an absent body — is success.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settled {
    pub settled: bool,
}

/// What a node holding somebody else's replicas says about them.
///
/// Sent by the node running them, on the same schedule it collects
/// errands on. The authority placed them and has no other way to know:
/// nothing can reach a private node to ask.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub replicas: Vec<ReplicaState>,
}

/// One copy, as the node running it sees it.
///
/// Named by project, service and **slot** rather than by an id: slot
/// numbers belong to the service, so this is one row on the other side
/// with nothing to map between. The names are the ones the errand
/// carried, so they resolve back on the node that sent them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaState {
    pub project: String,
    pub service: String,
    pub slot: u32,
    /// Where its container answers, while it is up.
    #[serde(default)]
    pub address: Option<String>,
    /// The port on that node's overlay address which reaches this
    /// copy. What an edge is given as an upstream — the container's own
    /// address is on a bridge that is not unique across nodes.
    #[serde(default)]
    pub overlay_port: Option<u16>,
    /// Why it is not, when it is not.
    #[serde(default)]
    pub error: Option<String>,
    /// The operator of that machine threw it out. The node that placed
    /// it stops asking — a danger zone the origin undid would not be
    /// one.
    #[serde(default)]
    pub evicted: bool,
}

/// A refusal, in the shape the other end knows how to print.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub error: String,
}

/// A body past this is not a node describing itself.
const MAX_BODY: usize = 64 * 1024;

#[injectable]
pub struct NetworkApi {
    database: Arc<SqliteDatabase>,
    /// For the overlay: a node that just joined is a peer, and the
    /// interface has to be told before anything can reach it.
    config: Arc<crate::config::Config>,
}

#[rest_controller("/api/network")]
impl NetworkApi {
    /// What this node has asked the caller to do.
    ///
    /// Collected rather than delivered: the authority cannot reach a
    /// private node over TLS — it has a certificate for a name, not for
    /// an overlay address — so the node that takes instructions is the
    /// one that dials, over the certificate it enrolled through. The
    /// authority still decides; the model is who gives orders, not who
    /// makes the call.
    ///
    /// Handing the same errand over twice is normal. A node that
    /// collected one and then died has to be given it again, and
    /// nothing here can tell that from a slow one — so what settles an
    /// errand is the acknowledgement below, not the collection.
    #[get("/errands")]
    #[raw]
    async fn errands(&self, request: Request) -> RestResult<Response> {
        // Copied before anything is awaited: see `asking`.
        let Some(secret) = bearer(&request).map(str::to_string) else {
            return Ok(refuse(
                StatusCode::UNAUTHORIZED,
                "this endpoint needs the secret from the token this node was enrolled with",
            ));
        };
        let node = match asking(&self.database, &secret).await {
            Ok(node) => node,
            Err(response) => return Ok(response),
        };

        match errand::waiting(&self.database, &node).await {
            Ok(waiting) => Ok(json(StatusCode::OK, &waiting)),
            Err(error) => Ok(unreadable(error)),
        }
    }

    /// How it went. Both outcomes end here.
    ///
    /// A failure is an answer — the authority learns what happened,
    /// with the reason — and the state this exists to prevent is an
    /// errand that stays pending for ever because nobody said.
    #[post("/errands/:errand/done")]
    #[raw]
    async fn settle(&self, request: Request) -> RestResult<Response> {
        // Copied before anything is awaited: see `asking`.
        let Some(secret) = bearer(&request).map(str::to_string) else {
            return Ok(refuse(
                StatusCode::UNAUTHORIZED,
                "this endpoint needs the secret from the token this node was enrolled with",
            ));
        };
        let node = match asking(&self.database, &secret).await {
            Ok(node) => node,
            Err(response) => return Ok(response),
        };
        let path = request.uri().path().to_string();
        // Four segments in, not two: this controller is mounted under
        // `/api/network`, so the path is `/api/network/errands/<id>/done`.
        let Some(id) = crate::console::auth::segments(&path)
            .get(3)
            .map(|id| id.to_string())
        else {
            return Ok(refuse(StatusCode::NOT_FOUND, "no such errand"));
        };

        let Ok(bytes) = wabot::rest::axum::body::to_bytes(request.into_body(), MAX_BODY).await
        else {
            return Ok(refuse(StatusCode::BAD_REQUEST, "that request is too large"));
        };
        // An empty body is success. Saying nothing went wrong by saying
        // nothing is the shape a caller reaches for, and refusing it
        // would make the common case the awkward one.
        let outcome: Outcome = match bytes.is_empty() {
            true => Outcome { error: None },
            false => match serde_json::from_slice(&bytes) {
                Ok(outcome) => outcome,
                Err(_) => return Ok(refuse(StatusCode::BAD_REQUEST, "that is not an outcome")),
            },
        };

        match errand::settle(&self.database, &node, &id, outcome.error.as_deref()).await {
            // Already settled, or not this node's: the same answer,
            // because from over there they are the same thing — there
            // is nothing of yours here by that name.
            Ok(false) => Ok(refuse(StatusCode::NOT_FOUND, "no such errand")),
            Ok(true) => {
                match &outcome.error {
                    Some(reason) => {
                        tracing::warn!(node = %node, errand = %id, %reason, "an errand failed")
                    }
                    None => tracing::info!(node = %node, errand = %id, "an errand was carried out"),
                }
                Ok(json(StatusCode::OK, &Settled { settled: true }))
            }
            Err(error) => Ok(unreadable(error)),
        }
    }

    /// What the caller says about the replicas this node placed there.
    ///
    /// The only way an authority learns anything about a copy it does
    /// not run: nothing can reach a private node to ask, so the node
    /// running it says so on the same schedule it collects errands on.
    ///
    /// Only rows this node placed **on that node** are touched. A
    /// report naming a replica somebody else holds, or one that runs
    /// here, is ignored rather than refused — the caller is describing
    /// its own machine and cannot know what this one has decided since.
    #[post("/report")]
    #[raw]
    async fn report(&self, request: Request) -> RestResult<Response> {
        let Some(secret) = bearer(&request).map(str::to_string) else {
            return Ok(refuse(
                StatusCode::UNAUTHORIZED,
                "this endpoint needs the secret from the token this node was enrolled with",
            ));
        };
        let node = match asking(&self.database, &secret).await {
            Ok(node) => node,
            Err(response) => return Ok(response),
        };

        let Ok(bytes) = wabot::rest::axum::body::to_bytes(request.into_body(), MAX_BODY).await
        else {
            return Ok(refuse(StatusCode::BAD_REQUEST, "that request is too large"));
        };
        let Ok(report) = serde_json::from_slice::<Report>(&bytes) else {
            return Ok(refuse(StatusCode::BAD_REQUEST, "that is not a report"));
        };

        let mut recorded = 0;
        for state in &report.replicas {
            match record(&self.database, &node, state).await {
                Ok(true) => recorded += 1,
                Ok(false) => {}
                Err(error) => return Ok(unreadable(error)),
            }
        }

        tracing::debug!(node = %node, recorded, "a node reported");
        Ok(json(StatusCode::OK, &Settled { settled: true }))
    }

    /// Record a node that is holding one of this node's join tokens.
    ///
    /// `#[raw]` because the answer is a status as much as a body: a
    /// refused token has to arrive as 401 so the other end can tell
    /// "you are not allowed" from "I could not read you".
    #[post("/join")]
    #[raw]
    async fn join(&self, request: Request) -> RestResult<Response> {
        // Copied rather than borrowed: reading the body consumes the
        // request, and the header has to outlive it.
        let Some(secret) = bearer(&request).map(str::to_string) else {
            return Ok(refuse(
                StatusCode::UNAUTHORIZED,
                "this endpoint needs the secret from a join token",
            ));
        };

        let Ok(bytes) = wabot::rest::axum::body::to_bytes(request.into_body(), MAX_BODY).await
        else {
            return Ok(refuse(StatusCode::BAD_REQUEST, "that request is too large"));
        };
        let Ok(arriving) = serde_json::from_slice::<Arriving>(&bytes) else {
            return Ok(refuse(
                StatusCode::BAD_REQUEST,
                "that is not a node describing itself",
            ));
        };
        if let Err(reason) = describes_a_node(&arriving) {
            return Ok(refuse(StatusCode::BAD_REQUEST, reason));
        }

        // Looked up before anything is written, and the same refusal
        // whether the token is unknown, expired or made up: telling a
        // caller which of the three it was tells them something about a
        // secret they do not hold.
        let enrolment = match enrolment::look_up(&self.database, &secret).await {
            Ok(Some(enrolment)) => enrolment,
            Ok(None) => {
                return Ok(refuse(
                    StatusCode::UNAUTHORIZED,
                    "that join token is not valid — it may have expired, or been withdrawn",
                ))
            }
            Err(error) => return Ok(unreadable(error)),
        };

        // What this node already knows by that id, if anything.
        //
        // Only its *own* id is a refusal. A node already known here
        // presenting a fresh token is a **re-join** — which is a thing
        // operators are told to do, and was refused as an id collision
        // until a real one was needed. The caller is holding a token
        // this node minted and has not spent, which is exactly the
        // standing required to claim an id.
        let known = match super::find(&self.database, &arriving.node).await {
            Ok(known) => known,
            Err(error) => return Ok(unreadable(error)),
        };
        if known.as_ref().is_some_and(|node| node.is_self) {
            return Ok(refuse(StatusCode::CONFLICT, "that id is this node's own"));
        }

        match enrolment::spend(&self.database, &enrolment.id, &arriving.node).await {
            Ok(true) => {}
            Ok(false) => {
                return Ok(refuse(
                    StatusCode::CONFLICT,
                    "that join token has already been used by another node",
                ))
            }
            Err(error) => return Ok(unreadable(error)),
        }

        let recorded = super::save(
            &self.database,
            &Node {
                id: arriving.node.clone(),
                // Whatever it calls itself. Cosmetic, and the enrolment
                // already carries what the operator called it — but a
                // list showing the node's own name is a list somebody
                // can match against the machine in front of them.
                name: arriving.name.clone(),
                // Private: it arrived through a token rather than being
                // dialled, and this node has no address for it except
                // the overlay one it just allocated.
                kind: Kind::Private,
                endpoint: None,
                public_key: Some(arriving.public_key.clone()),
                // A node that is already here keeps the address it
                // already has. Re-joining used to move it, which meant
                // reconfiguring both ends of a working tunnel and
                // leaking the old address for ever — a spent enrolment
                // holds its address, so nothing ever gave it back.
                overlay_ip: Some(
                    known
                        .as_ref()
                        .and_then(|node| node.overlay_ip.clone())
                        .unwrap_or_else(|| enrolment.overlay_ip.clone()),
                ),
                is_self: false,
                last_seen_at: Some(now_ms()),
            },
        )
        .await;
        if let Err(error) = recorded {
            return Ok(unreadable(error));
        }

        let me = match super::me(&self.database).await {
            Ok(Some(me)) => me,
            // Only reachable on a node that has never started properly
            // — `serve` seeds this row — and answering with somebody
            // else's identity would be worse than answering with none.
            Ok(None) => {
                tracing::error!("a node joined but this one has no row of its own");
                return Ok(refuse(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "this node does not know what it is yet",
                ));
            }
            Err(error) => return Ok(unreadable(error)),
        };

        // Same reason as the other side: the peer set changed. The
        // join has already succeeded, so a kernel that refuses the
        // interface is reported and not answered with a refusal.
        if let Err(error) = super::tunnel::ensure(&self.database, &self.config).await {
            tracing::warn!(%error, "a node joined, but the overlay did not come up");
        }

        tracing::info!(
            node = %arriving.node,
            name = %arriving.name,
            overlay_ip = %enrolment.overlay_ip,
            "a node joined"
        );

        Ok(json(
            StatusCode::OK,
            &Accepted {
                authority: me.id,
                name: me.name,
                overlay_ip: known
                    .and_then(|node| node.overlay_ip)
                    .unwrap_or(enrolment.overlay_ip),
            },
        ))
    }
}

/// The fields that have to be there for any of this to mean anything.
///
/// Checked here rather than trusted, because everything below writes
/// them into a row: an empty id would be a node nothing can name, and a
/// three-kilobyte one would be a table nobody can read.
fn describes_a_node(arriving: &Arriving) -> Result<(), &'static str> {
    let id_is_sane = !arriving.node.is_empty()
        && arriving.node.len() <= 64
        && arriving
            .node
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !id_is_sane {
        return Err("that is not a node id");
    }
    if arriving.name.is_empty() || arriving.name.len() > 200 {
        return Err("a node has to have a name");
    }
    if arriving.public_key.is_empty() || arriving.public_key.len() > 200 {
        return Err("a node has to bring a public key");
    }
    Ok(())
}

/// Which node is asking, from the secret it presented.
///
/// Three refusals collapse into one answer on purpose — an unknown
/// secret, a secret nobody has spent, and a node this one has since
/// forgotten all read as "not yours". Telling them apart would tell a
/// caller something about a secret it does not hold, and the third is
/// what makes *Forget* actually cut a node off rather than merely
/// hiding it from a page.
/// Takes the secret rather than the request: a `&Request` held across
/// an await makes the handler's future non-`Send`, because axum's
/// `Body` is not `Sync`. Reading the header first is not a style
/// choice.
async fn asking(database: &SqliteDatabase, secret: &str) -> Result<String, Response> {
    let refused = || {
        refuse(
            StatusCode::UNAUTHORIZED,
            "this endpoint needs the secret from the token this node was enrolled with",
        )
    };

    let node = match enrolment::holder(database, secret).await {
        Ok(Some(node)) => node,
        Ok(None) => return Err(refused()),
        Err(error) => return Err(unreadable(error)),
    };
    match super::find(database, &node).await {
        Ok(Some(_)) => Ok(node),
        Ok(None) => Err(refused()),
        Err(error) => Err(unreadable(error)),
    }
}

/// The secret out of an `Authorization: Bearer` header.
fn bearer(request: &Request) -> Option<&str> {
    let value = request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?
        .trim();
    (!value.is_empty()).then_some(value)
}

fn json<T: Serialize>(status: StatusCode, value: &T) -> Response {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("a serialised body is well-formed")
}

/// Write one reported state onto the row this node placed.
///
/// `false` when there is nothing here it matches — a service renamed
/// since, or a replica this node has already moved. Not an error: the
/// other end is describing its own machine truthfully, and it cannot
/// know what has been decided here since it was told.
async fn record(
    database: &SqliteDatabase,
    node: &str,
    state: &ReplicaState,
) -> Result<bool, super::NetworkError> {
    let Some(project) = crate::platform::projects::all(database)
        .await
        .map_err(|error| super::NetworkError::Refused(error.to_string()))?
        .into_iter()
        .find(|project| project.name == state.project)
    else {
        return Ok(false);
    };
    let found = crate::platform::services::in_project(
        database,
        &project.id,
        &crate::platform::slugify(&state.service),
    )
    .await
    .map_err(|error| super::NetworkError::Refused(error.to_string()))?;
    let Some(service) = found else {
        return Ok(false);
    };

    let replica = crate::platform::replicas::in_slot(database, &service.id, state.slot)
        .await
        .map_err(|error| super::NetworkError::Refused(error.to_string()))?;
    // Only what this node placed *there*. A row that has since been
    // brought home, or moved to a third node, is not this caller's to
    // describe any more.
    let Some(replica) = replica.filter(|replica| replica.node_id.as_deref() == Some(node)) else {
        return Ok(false);
    };

    let write = async {
        if state.evicted {
            crate::platform::replicas::evict(database, &replica.id).await?;
            return Ok::<(), crate::platform::PlatformError>(());
        }
        crate::platform::replicas::set_address(database, &replica.id, state.address.as_deref())
            .await?;
        crate::platform::replicas::set_overlay_port(database, &replica.id, state.overlay_port)
            .await?;
        crate::platform::replicas::set_last_error(database, &replica.id, state.error.as_deref())
            .await
    };
    write
        .await
        .map_err(|error| super::NetworkError::Refused(error.to_string()))?;
    Ok(true)
}

/// A storage failure, in the same shape as every other answer here.
///
/// The crate's own error body would be a different shape, and the node
/// on the other end reads this one — a refusal it cannot parse is a
/// status code and nothing else. 503 because the right move from over
/// there is to try the same token again.
fn unreadable(error: super::NetworkError) -> Response {
    tracing::error!(%error, "a node tried to join and this one could not read its own records");
    refuse(
        StatusCode::SERVICE_UNAVAILABLE,
        "this node could not read its own records; try again",
    )
}

fn refuse(status: StatusCode, reason: &str) -> Response {
    json(
        status,
        &Refusal {
            error: reason.to_string(),
        },
    )
}

/// Nothing is discovered: a type left out here panics on resolve.
pub fn register(container: &Container, config: crate::config::Config) {
    container.register_instance::<crate::config::Config>(Arc::new(config));
    register_transients!(container, NetworkApi);
}

pub fn routes(container: &Container) -> Router {
    NetworkApi::register_routes(container, Router::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use wabot::testing::RestHarness;

    /// A public node with one enrolment outstanding, and the token that
    /// spends it.
    struct Authority {
        harness: RestHarness,
        database: Arc<SqliteDatabase>,
        secret: String,
        me: String,
    }

    async fn authority() -> Authority {
        let database = Arc::new(crate::db::open_in_memory().await.expect("open"));
        let admin = crate::network::tests::admin(&database).await;
        crate::node::settings::set_domain(&database, Some("hub.example"))
            .await
            .expect("domain");
        let me = super::super::ensure_self(&database, &Config::default())
            .await
            .expect("seeded");

        let address = super::super::overlay::allocate(&database)
            .await
            .expect("allocate");
        let (_, secret) = enrolment::create(&database, "alpine", &address, &admin)
            .await
            .expect("minted");

        let container = Container::new();
        container.register_instance::<SqliteDatabase>(database.clone());
        register(&container, Config::default());

        Authority {
            harness: RestHarness::new(routes(&container)),
            database,
            secret,
            me: me.id,
        }
    }

    fn arriving() -> serde_json::Value {
        serde_json::json!({
            "node": "nd-joining001",
            "name": "alpine.example",
            "public_key": "0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=",
        })
    }

    /// The whole of phase 1 in one request: a node holding a token
    /// becomes a row here, on the address that was allocated for it.
    #[tokio::test]
    async fn a_node_holding_a_token_becomes_a_node() {
        let authority = authority().await;

        let response = authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await;
        response.assert_ok();

        let accepted = response.value();
        assert_eq!(accepted["authority"], authority.me);
        assert_eq!(accepted["overlay_ip"], "10.42.0.1");

        let joined = super::super::find(&authority.database, "nd-joining001")
            .await
            .expect("query")
            .expect("recorded");
        assert_eq!(joined.kind, Kind::Private);
        assert_eq!(joined.overlay_ip.as_deref(), Some("10.42.0.1"));
        assert!(joined.public_key.is_some());
        assert!(
            joined.last_seen_at.is_some(),
            "a join is this node hearing from it"
        );
        assert!(
            !joined.may_be_edge(),
            "it has no address the world can dial"
        );
    }

    /// A response that never arrived is re-sent, and the same node
    /// arriving twice is one join.
    #[tokio::test]
    async fn the_same_node_may_arrive_twice() {
        let authority = authority().await;

        for _ in 0..2 {
            authority
                .harness
                .post("/api/network/join")
                .header("authorization", format!("Bearer {}", authority.secret))
                .json(&arriving())
                .send()
                .await
                .assert_ok();
        }

        assert_eq!(
            super::super::all(&authority.database)
                .await
                .expect("query")
                .len(),
            2,
            "this node and the one that joined it"
        );
    }

    /// The property the token exists for.
    #[tokio::test]
    async fn a_second_node_cannot_spend_the_same_token() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        let mut second = arriving();
        second["node"] = serde_json::json!("nd-joining002");
        let response = authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&second)
            .send()
            .await;

        response.assert_status(StatusCode::CONFLICT);
        assert_eq!(
            super::super::find(&authority.database, "nd-joining002")
                .await
                .expect("query"),
            None
        );
    }

    /// The upsert underneath would let a claimed id overwrite the row
    /// it collided with — including the row that says which node this
    /// is. A claim is refused rather than merged.
    #[tokio::test]
    async fn a_node_may_not_claim_an_id_this_one_is_using() {
        let authority = authority().await;

        let mut claiming = arriving();
        claiming["node"] = serde_json::json!(authority.me.clone());
        let response = authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&claiming)
            .send()
            .await;

        response.assert_status(StatusCode::CONFLICT);
        assert!(
            response.body.contains("this node's own"),
            "and says which way it collided: {}",
            response.body
        );

        let me = super::super::me(&authority.database)
            .await
            .expect("query")
            .expect("still here");
        assert!(me.is_self);
        assert_eq!(me.kind, Kind::Public, "its own row was overwritten");
    }

    /// A refusal must not cost the token: the operator can fix a bad
    /// body, and cannot fix a token that was burnt on one.
    #[tokio::test]
    async fn a_refused_request_does_not_spend_the_token() {
        let authority = authority().await;

        let mut nameless = arriving();
        nameless["name"] = serde_json::json!("");
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&nameless)
            .send()
            .await
            .assert_status(StatusCode::BAD_REQUEST);

        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();
    }

    /// The only way an authority learns anything about a copy it does
    /// not run. Nothing can reach a private node to ask, so the node
    /// running it says so — and until it does, the page says "waiting"
    /// rather than inventing an outcome.
    #[tokio::test]
    async fn a_node_reports_what_it_is_running_for_this_one() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        let placed = placed_there(&authority, 2).await;

        authority
            .harness
            .post("/api/network/report")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serde_json::json!({
                "replicas": [
                    { "project": "shared", "service": "web", "slot": 2,
                      "address": "10.42.7.3" }
                ]
            }))
            .send()
            .await
            .assert_ok();

        let after = crate::platform::replicas::in_slot(&authority.database, &placed, 2)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(after.address.as_deref(), Some("10.42.7.3"));
        assert!(!after.evicted());
    }

    /// An eviction is the one thing the node running a replica can
    /// always do, and the authority has to stop asking — a danger zone
    /// the origin undid would not be one.
    #[tokio::test]
    async fn an_eviction_reaches_the_node_that_placed_it() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();
        let placed = placed_there(&authority, 2).await;

        authority
            .harness
            .post("/api/network/report")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serde_json::json!({
                "replicas": [
                    { "project": "shared", "service": "web", "slot": 2, "evicted": true }
                ]
            }))
            .send()
            .await
            .assert_ok();

        let after = crate::platform::replicas::in_slot(&authority.database, &placed, 2)
            .await
            .expect("query")
            .expect("there");
        assert!(after.evicted(), "the authority kept asking for it");
    }

    /// A node describes its own machine truthfully and cannot know what
    /// has been decided here since. A report naming a replica this node
    /// has brought home is ignored rather than refused — and must not
    /// overwrite the row that is now local.
    #[tokio::test]
    async fn a_report_about_a_replica_that_came_home_is_ignored() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();
        let placed = placed_there(&authority, 2).await;

        let replica = crate::platform::replicas::in_slot(&authority.database, &placed, 2)
            .await
            .expect("query")
            .expect("there");
        crate::platform::replicas::move_to(&authority.database, &replica.id, None)
            .await
            .expect("home");

        authority
            .harness
            .post("/api/network/report")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serde_json::json!({
                "replicas": [
                    { "project": "shared", "service": "web", "slot": 2, "evicted": true }
                ]
            }))
            .send()
            .await
            .assert_ok();

        let after = crate::platform::replicas::in_slot(&authority.database, &placed, 2)
            .await
            .expect("query")
            .expect("there");
        assert!(!after.evicted(), "a stale report evicted a local replica");
        assert!(after.is_here());
    }

    /// A service with a replica placed on the joined node, and its id.
    async fn placed_there(authority: &Authority, slot: u32) -> String {
        let project = crate::platform::projects::create(&authority.database, "shared")
            .await
            .expect("project");
        let service = crate::platform::services::create(
            &authority.database,
            &project.id,
            "web",
            "hub.example/shared/web@sha256:abc",
            &[],
        )
        .await
        .expect("service");
        crate::platform::replicas::place(
            &authority.database,
            &service.id,
            Some("nd-joining001"),
            slot,
        )
        .await
        .expect("placed");
        service.id
    }

    /// The whole of phase 3's delivery, from the authority's side: a
    /// node asks what is waiting for it, does it, and says how it went.
    #[tokio::test]
    async fn a_node_collects_its_errands_and_reports_back() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        errand::queue(
            &authority.database,
            "nd-joining001",
            errand::Kind::Host,
            &serde_json::json!({ "service": "web" }),
        )
        .await
        .expect("queued");

        let response = authority
            .harness
            .get("/api/network/errands")
            .header("authorization", format!("Bearer {}", authority.secret))
            .send()
            .await;
        response.assert_ok();
        let waiting = response.value();
        assert_eq!(waiting.as_array().map(Vec::len), Some(1));
        let id = waiting[0]["id"].as_str().expect("an id").to_string();
        assert_eq!(waiting[0]["kind"], "host");

        // An empty body is success — saying nothing went wrong by
        // saying nothing is the shape a caller reaches for.
        authority
            .harness
            .post(&format!("/api/network/errands/{id}/done"))
            .header("authorization", format!("Bearer {}", authority.secret))
            .send()
            .await
            .assert_ok();

        let settled = authority
            .harness
            .get("/api/network/errands")
            .header("authorization", format!("Bearer {}", authority.secret))
            .send()
            .await;
        assert_eq!(settled.value().as_array().map(Vec::len), Some(0));

        let record = errand::find(&authority.database, &id)
            .await
            .expect("query")
            .expect("there");
        assert!(record.done() && !record.failed());
    }

    /// A failure is an answer. The state this prevents is an errand
    /// pending for ever because nobody recorded what happened.
    #[tokio::test]
    async fn a_failure_carries_its_reason_back() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();
        let queued = errand::queue(
            &authority.database,
            "nd-joining001",
            errand::Kind::Host,
            &serde_json::json!({}),
        )
        .await
        .expect("queued");

        authority
            .harness
            .post(&format!("/api/network/errands/{}/done", queued.id))
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serde_json::json!({ "error": "no such image" }))
            .send()
            .await
            .assert_ok();

        let record = errand::find(&authority.database, &queued.id)
            .await
            .expect("query")
            .expect("there");
        assert!(record.done());
        assert_eq!(record.error.as_deref(), Some("no such image"));
    }

    /// Forgetting a node has to cut it off, not merely hide it from a
    /// page. Its credential still exists — the enrolment row that named
    /// it is untouched — so the check is that the node is still known.
    #[tokio::test]
    async fn a_forgotten_node_is_refused() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        super::super::forget(&authority.database, "nd-joining001")
            .await
            .expect("forget");

        authority
            .harness
            .get("/api/network/errands")
            .header("authorization", format!("Bearer {}", authority.secret))
            .send()
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    /// One node must not settle another's errand, and a secret nobody
    /// has spent is nobody.
    #[tokio::test]
    async fn errands_need_the_secret_of_the_node_they_belong_to() {
        let authority = authority().await;

        for header in [None, Some("Bearer made-up".to_string())] {
            let mut request = authority.harness.get("/api/network/errands");
            if let Some(header) = header {
                request = request.header("authorization", header);
            }
            request.send().await.assert_status(StatusCode::UNAUTHORIZED);
        }

        // Minted but never spent: a valid secret that names no node.
        authority
            .harness
            .get("/api/network/errands")
            .header("authorization", format!("Bearer {}", authority.secret))
            .send()
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    /// Nothing may write a row here without a token, and an unknown
    /// token is refused the same way an expired one is — which of the
    /// two it was is something about a secret the caller does not hold.
    #[tokio::test]
    async fn nothing_joins_without_a_token() {
        let authority = authority().await;

        for header in [None, Some("Bearer made-up"), Some("Basic whatever")] {
            let mut request = authority
                .harness
                .post("/api/network/join")
                .json(&arriving());
            if let Some(header) = header {
                request = request.header("authorization", header);
            }
            request.send().await.assert_status(StatusCode::UNAUTHORIZED);
        }

        assert_eq!(
            super::super::all(&authority.database)
                .await
                .expect("query")
                .len(),
            1,
            "only this node"
        );
    }
}
