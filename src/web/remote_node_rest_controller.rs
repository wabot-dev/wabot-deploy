//! What one node's routes look like to another.
//!
//! The shapes that travel — what arrives, what is accepted, what is
//! reported — stay in `api`, because the other side of the wire builds
//! them too.

use crate::infra::network::api::*;

use crate::infra::clock::now_ms;
use crate::infra::network::{enrolment, errand, Kind, Node};
use std::sync::Arc;
use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::axum::Router;
use wabot::rest::RestResult;
use wabot::sqlite::SqliteDatabase;

#[injectable]
pub struct RemoteNodeRestController {
    database: Arc<SqliteDatabase>,
    /// For the overlay: a node that just joined is a peer, and the
    /// interface has to be told before anything can reach it.
    config: Arc<crate::infra::config::Config>,
    /// For the routes. A report is the only way this node learns the
    /// port a copy elsewhere answers on, so it is also the moment the
    /// name pointing at that copy becomes routable — and nothing else
    /// would notice: the deployment that usually recomputes routes runs
    /// on the *other* node.
    deployer: Arc<crate::infra::deploy::Deployer>,
    /// Rung by an authority with something to hand over; answered by the
    /// collector loop.
    doorbell: Arc<crate::infra::network::collect::Doorbell>,
}

#[rest_controller("/api/network")]
impl RemoteNodeRestController {
    /// Come and ask me: an authority saying an errand is waiting.
    ///
    /// The one endpoint here that is called *by* an authority rather than
    /// by a node that takes orders from one, and it exists because the
    /// premise below turned out to be false — see `docs/network.md` phase
    /// 9, and `collect::Doorbell` for what it carries and does not.
    ///
    /// **Authenticated now**, which phase 9 wrote down as owed and could
    /// not build: there was no credential for the direction that dials
    /// *in*. There is one — see migration `0049` — so the caller says who
    /// it is instead of the path claiming it.
    ///
    /// A node too old to present one is refused, and its rings stop
    /// shortening the wait. That is the whole cost: the pull loop delivers
    /// everything within fifteen seconds and always did.
    ///
    /// The id in the path is now only the log's second opinion. It is kept
    /// because the URL shape is what an older authority calls, and because
    /// a mismatch between it and the credential is worth seeing.
    #[post("/wake/:authority")]
    #[raw]
    async fn wake(&self, request: Request) -> RestResult<Response> {
        let claimed = request
            .uri()
            .path()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();
        let Some(secret) = bearer(&request).map(str::to_string) else {
            tracing::debug!(%claimed, "rung by somebody presenting nothing");
            return Ok(refuse(
                StatusCode::UNAUTHORIZED,
                "this endpoint needs a secret this node knows you by",
            ));
        };
        // Either credential, because either side may be doing the ringing:
        // an authority with an errand for this node, or a node that has
        // just handed one *up* to it. That second one never worked — the
        // check here was "is the caller an authority", and a member is
        // not, so phase 10's handed-up errands rang and were declined.
        let node = match caller(&self.database, &secret).await {
            Ok(node) => node,
            Err(response) => return Ok(response),
        };

        self.doorbell.ring();
        tracing::debug!(%node, %claimed, "rung");
        Ok(json(StatusCode::OK, &Settled { settled: true }))
    }

    /// Serve a challenge answer for a name this node is a co-edge of.
    ///
    /// **Why this exists.** A public authority fetches an HTTP-01 token
    /// from whichever address the name resolves to, and the token lives in
    /// the ordering node's own table. With two nodes answering for one
    /// name, the authority has a fair chance of asking the one that does
    /// not have it — one of five failed validations an hour, against an
    /// account whose lock takes every other name on the node with it. So
    /// the node that ordered publishes the token here first.
    ///
    /// **Authenticated, and it has to be.** Everything else called by a
    /// node over the overlay is either bearing a credential it enrolled
    /// with or harmless — the doorbell asks this node to do what it was
    /// going to do anyway. This one is not harmless: somebody who could
    /// plant a response would get a certificate for a name this node
    /// serves. The secret is per hostname, minted by the node that owns
    /// the service and handed to every edge of it on the errand that made
    /// them one.
    ///
    /// Compared in constant time, and only against the secret for the
    /// name in the body: a secret for one name proves nothing about
    /// another.
    #[post("/challenge")]
    #[raw]
    async fn challenge(&self, request: Request) -> RestResult<Response> {
        let presented = request
            .headers()
            .get(wabot::rest::axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or_default()
            .to_string();

        let Ok(bytes) = wabot::rest::axum::body::to_bytes(request.into_body(), 8 * 1024).await
        else {
            return Ok(json(StatusCode::BAD_REQUEST, &Settled { settled: false }));
        };
        let Ok(asked) = serde_json::from_slice::<Challenge>(&bytes) else {
            return Ok(json(StatusCode::BAD_REQUEST, &Settled { settled: false }));
        };

        let held = crate::infra::repositories::edges::secret_for(&self.database, &asked.hostname)
            .await
            .ok()
            .flatten();
        let Some(held) = held else {
            // Nothing to compare against is a refusal, not an error: a
            // name this node is not a co-edge of is one it will not
            // answer a challenge for.
            tracing::debug!(hostname = %asked.hostname, "a challenge for a name with no relay secret");
            return Ok(json(StatusCode::FORBIDDEN, &Settled { settled: false }));
        };
        if !constant_time_eq(held.as_bytes(), presented.as_bytes()) {
            tracing::warn!(hostname = %asked.hostname, "a challenge presented the wrong secret");
            return Ok(json(StatusCode::FORBIDDEN, &Settled { settled: false }));
        }

        match crate::infra::edge::acme::store_challenge(
            &self.database,
            &asked.token,
            &asked.response,
            &asked.hostname,
        )
        .await
        {
            Ok(()) => {
                tracing::info!(
                    hostname = %asked.hostname,
                    token = %asked.token,
                    "answering an http-01 challenge for a co-edge"
                );
                Ok(json(StatusCode::OK, &Settled { settled: true }))
            }
            Err(error) => {
                tracing::warn!(%error, "could not store a co-edge's challenge");
                Ok(json(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &Settled { settled: false },
                ))
            }
        }
    }

    /// What this node has asked the caller to do.
    ///
    /// Collected rather than delivered, and that is still the model: the
    /// node that takes instructions is the one that dials, over the
    /// certificate it enrolled through, and writes its own rows from what
    /// it is told. What changed is only *when* it finds out — the
    /// doorbell above — because the reason given here for the pull was
    /// that an authority cannot reach a private node, and it can. See
    /// `docs/network.md` phase 9.
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

    /// Who else is on this network.
    ///
    /// **The one thing the star was missing.** `tunnel::peers` already
    /// builds a peer for every row this node has; a member only ever had a
    /// row for its authority, so two nodes enrolled by the same one had
    /// never heard of each other and had no session. Three features have
    /// worked around that by hand — a standby's primary, a hostname's
    /// co-edges, an errand handed upwards — and this is the answer they
    /// were each distributing a slice of.
    ///
    /// Its own endpoint rather than a field on the errands answer:
    /// `call::errands` deserialises a bare list, so wrapping it would
    /// break the wire in both directions during a mixed-version window —
    /// and an errand is *consumed and settled* where a roster is state
    /// that is read again every pass. A node that cannot read this still
    /// collects its errands.
    ///
    /// Same credential and same guard as those, so there is no new trust:
    /// what it discloses is who else this node enrolled, to a node it
    /// enrolled.
    #[get("/roster")]
    #[raw]
    async fn roster(&self, request: Request) -> RestResult<Response> {
        // Copied before anything is awaited: see `asking`.
        let Some(secret) = bearer(&request).map(str::to_string) else {
            return Ok(refuse(
                StatusCode::UNAUTHORIZED,
                "this endpoint needs the secret from the token this node was enrolled with",
            ));
        };
        if let Err(response) = asking(&self.database, &secret).await {
            return Ok(response);
        }

        match crate::infra::network::roster::of_this_node(&self.database).await {
            Ok(roster) => Ok(json(StatusCode::OK, &roster)),
            Err(error) => Ok(unreadable(error)),
        }
    }

    /// An errand the other way round: a node this one enrolled asking
    /// *it* to do something.
    ///
    /// **The direction the model did not have.** Collection asks a
    /// node's authorities, so an errand addressed to an authority was
    /// asked for by nobody — it sat pending for ever and the thing it
    /// asked for never happened. That is not a rare corner: being an
    /// edge is a row like any other, so a service owned by a joined node
    /// and served by the node that enrolled it produces one of these
    /// every time. There is one on the Alpine test node, five days old,
    /// for a name the Ubuntu node was never told to answer for.
    ///
    /// Queued here, not carried out here. This is a request that can
    /// time out and be retried, and obeying an errand writes rows and
    /// starts deployments — so it goes on this node's own queue,
    /// addressed to this node, with the asker recorded beside it, and
    /// the pass that already carries out this node's own errands picks
    /// it up. Obeying is local, which is what this has said since phase
    /// 3.
    ///
    /// The same errand arriving twice is normal and harmless for the
    /// same reason it is on the collecting side: an errand is a
    /// convergent instruction, and the sender cannot tell a lost reply
    /// from a slow one.
    #[post("/asked")]
    #[raw]
    async fn asked(&self, request: Request) -> RestResult<Response> {
        // Copied before anything is awaited: see `asking`.
        let Some(secret) = bearer(&request).map(str::to_string) else {
            return Ok(refuse(
                StatusCode::UNAUTHORIZED,
                "this endpoint needs the secret from the token this node was enrolled with",
            ));
        };
        let from = match asking(&self.database, &secret).await {
            Ok(node) => node,
            Err(response) => return Ok(response),
        };

        let Ok(bytes) = wabot::rest::axum::body::to_bytes(request.into_body(), MAX_BODY).await
        else {
            return Ok(refuse(StatusCode::BAD_REQUEST, "that request is too large"));
        };
        let Ok(asked) = serde_json::from_slice::<Asked>(&bytes) else {
            return Ok(refuse(StatusCode::BAD_REQUEST, "that is not an errand"));
        };

        // Refused by name rather than stored and failed later. A node
        // newer than this one asking for something it has never heard of
        // learns that here, in the answer, rather than by watching an
        // errand fail on a machine it cannot see.
        let kind = errand::Kind::parse(&asked.kind);
        if kind == errand::Kind::Unknown {
            return Ok(refuse(
                StatusCode::BAD_REQUEST,
                "this node is older than that instruction and does not know it",
            ));
        }

        let me = match crate::infra::network::me(&self.database).await {
            Ok(Some(me)) => me.id,
            // A node with no row of its own has never been on a network
            // and cannot have enrolled the caller — so this is
            // unreachable in practice, and answering it as a refusal
            // beats unwrapping.
            Ok(None) => {
                return Ok(refuse(
                    StatusCode::CONFLICT,
                    "this node has no identity of its own yet",
                ))
            }
            Err(error) => return Ok(unreadable(error)),
        };

        // **Consent is checkable at the door, so it is checked here.**
        // Obeying stays deferred — a request can time out and be retried,
        // and obeying writes rows and starts deployments — but *refusing
        // for lack of permission* needs nothing but a local read, and
        // putting it in the answer is what makes it reach the node that
        // asked.
        //
        // Measured on the real network: an edge errand was accepted, then
        // refused by `carry_out` for a grant that had never been written,
        // and the refusal stayed in a row on this side. The asker's own
        // copy said "delivered" — `hand_over` settles on a 200 — so
        // fixing the grant changed nothing and nothing was ever sent
        // again. A refusal in the answer leaves the errand pending there,
        // and the next pass tries it.
        if let Some(needed) = errand::consent_for(kind, &asked.payload) {
            if let Err(reason) =
                crate::infra::network::collect::allowed(&self.database, &from, needed).await
            {
                tracing::info!(from = %from, kind = %asked.kind, %reason, "refused an errand at the door");
                return Ok(refuse(StatusCode::FORBIDDEN, &reason));
            }
        }

        match errand::accept(&self.database, &me, &from, kind, &asked.payload).await {
            Ok(errand) => {
                tracing::info!(from = %from, kind = %asked.kind, errand = %errand.id,
                    "a node asked this one to do something");
                // Its own doorbell, so this happens now rather than
                // within fifteen seconds. The caller is waiting to hear
                // that it landed, not that it is done — and the loop is
                // what does it.
                self.doorbell.ring();
                Ok(json(StatusCode::ACCEPTED, &Settled { settled: true }))
            }
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
        let Some(id) = crate::web::console::auth::segments(&path)
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

    /// What a copy of your service is saying, on the machine running it.
    ///
    /// **The one thing the logs page has always admitted it could not
    /// do.** A replica writes its log on the disk of the node that runs
    /// it, and until phase 9 there was no way to ask — which is why that
    /// page says "open the console of the node holding it".
    ///
    /// Reachable by either credential, because either node may be the one
    /// holding the copy: an authority dials its member over the overlay,
    /// a member dials its authority's public endpoint. See [`caller`].
    ///
    /// ## What guards it
    ///
    /// One local read: the service's `origin_node_id`, which is the node
    /// that placed it. Not a capability of its own — a log comes with
    /// `host`, the way the registry pull does, because a node that agreed
    /// to run your containers cannot sensibly refuse you what they write.
    /// And not the grant table either: the grant answers "may they ask me
    /// to run this", where the question here is "is this theirs", and the
    /// rows are the only thing that knows.
    #[post("/replica-log")]
    #[raw]
    async fn replica_log(&self, request: Request) -> RestResult<Response> {
        let Some(secret) = bearer(&request).map(str::to_string) else {
            return Ok(refuse(
                StatusCode::UNAUTHORIZED,
                "this endpoint needs a secret this node knows you by",
            ));
        };
        let node = match caller(&self.database, &secret).await {
            Ok(node) => node,
            Err(response) => return Ok(response),
        };
        let Ok(bytes) = wabot::rest::axum::body::to_bytes(request.into_body(), MAX_BODY).await
        else {
            return Ok(refuse(StatusCode::BAD_REQUEST, "that request is too large"));
        };
        let Ok(asked) = serde_json::from_slice::<LogWanted>(&bytes) else {
            return Ok(refuse(StatusCode::BAD_REQUEST, "that is not a log request"));
        };

        // By **name**, which is the vocabulary both ends already share:
        // an errand carries the project's and the service's names, a
        // report names a replica by them, and neither node has ever seen
        // the other's ids.
        let projects = match crate::infra::repositories::projects::all(&self.database).await {
            Ok(projects) => projects,
            Err(error) => return Ok(unreadable_platform(error)),
        };
        let Some(project) = projects
            .into_iter()
            .find(|project| project.name == asked.project)
        else {
            return Ok(refuse(StatusCode::NOT_FOUND, "no such project here"));
        };
        let services = match crate::infra::repositories::services::all(
            &self.database,
            Some(&project.id),
        )
        .await
        {
            Ok(services) => services,
            Err(error) => return Ok(unreadable_platform(error)),
        };
        let Some(service) = services
            .into_iter()
            .find(|service| service.name == asked.service)
        else {
            return Ok(refuse(StatusCode::NOT_FOUND, "no such service here"));
        };
        if service.origin_node_id.as_deref() != Some(node.as_str()) {
            // Said rather than answered with a 404: "that is not yours"
            // is a different thing from "there is nothing here", and the
            // node asking can act on the first.
            return Ok(refuse(
                StatusCode::FORBIDDEN,
                "that service was not placed here by you",
            ));
        }

        let container = crate::domain::entities::replica::container_id_for(
            &project.slug,
            &service.slug,
            asked.slot,
        );
        let data_dir = &self.config.node.data_dir;
        // The whole kept history, or the live file. The same two readings
        // the local page offers, asked for the same way.
        let text = match asked.all {
            true => crate::infra::deploy::logs::history(data_dir, &container, MAX_LOG)
                .or_else(|| crate::infra::deploy::logs::tail(data_dir, &container, MAX_LOG)),
            false => crate::infra::deploy::logs::tail(data_dir, &container, MAX_LOG),
        };
        match text {
            Some(text) => Ok(Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(Body::from(text))
                .unwrap_or_else(|_| refuse(StatusCode::INTERNAL_SERVER_ERROR, "could not answer"))),
            // A copy that has never written anything, or one started
            // before its output was being kept. Not an error, and the
            // difference between the two is not this node's to explain —
            // the page that asked says it.
            None => Ok(refuse(StatusCode::NOT_FOUND, "no log for that copy here")),
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

        // Before the replicas, because it is about the node itself and
        // a failure to read one replica must not lose it.
        // Its own addresses, before the reachability write and beside it:
        // the column is outside `Node` on purpose, so it has a statement
        // of its own — see migration `0046`.
        // What this node may present when it dials that one back. Its own
        // statement, like the addresses below and for the same reason:
        // a credential must not be something a row rebuilt from a report
        // can take away as a side effect.
        if let Some(secret) = report.dial.as_deref().filter(|secret| !secret.is_empty()) {
            if let Err(error) =
                crate::infra::network::remember_dial_secret(&self.database, &node, secret).await
            {
                tracing::warn!(%error, node = %node, "could not record how to dial a node back");
            }
        }
        if let Some(addresses) = &report.lan {
            if let Err(error) =
                crate::infra::network::lan::remember(&self.database, &node, addresses).await
            {
                tracing::warn!(%error, node = %node, "could not record a node's own addresses");
            }
        }
        if let Err(error) = note_reachability(&self.database, &node, &report).await {
            return Ok(unreadable(error));
        }

        let mut recorded = 0;
        for state in &report.replicas {
            match record(&self.database, &node, state).await {
                Ok(true) => recorded += 1,
                Ok(false) => {}
                Err(error) => return Ok(unreadable(error)),
            }
        }

        // And what it says it is holding that this node does not claim.
        //
        // A copy whose row went away here goes on running there for ever:
        // the instruction that would stop it is queued by whatever made
        // the change, and it cannot be queued if the change already
        // happened, if this node was down when it did, or if the code that
        // should have sent it did not — all three have occurred.
        //
        // So this is the convergent half, and it asks about now: that node
        // reports slot 3 of a service this node owns, this node's rows say
        // slot 3 is somewhere else or gone, and the answer is to send the
        // placement again — which is computed from the rows and is
        // therefore right whatever the disagreement was. Not a bespoke
        // "drop it": a node holding two copies where this node claims one
        // must keep the one it should have.
        for service in unclaimed(&self.database, &node, &report.replicas).await {
            let Some(project) =
                crate::infra::repositories::projects::find(&self.database, &service.project_id)
                    .await
                    .ok()
                    .flatten()
            else {
                continue;
            };
            let also: std::collections::BTreeSet<String> = [node.clone()].into_iter().collect();
            tracing::info!(
                node = %node,
                service = %service.slug,
                "it holds a copy this node does not claim; sending the placement again"
            );
            match service.kind.is_managed() {
                true => {
                    self.deployer
                        .dispatch_standbys_including(&service, &also)
                        .await
                }
                false => {
                    self.deployer
                        .tell_holders_including(&project, &service, &also)
                        .await
                }
            }
        }

        // Only when something moved, so a node saying the same thing
        // every fifteen seconds does not rebuild the table every
        // fifteen seconds.
        if recorded > 0 {
            self.deployer.sync_routes().await;
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
        let known = match crate::infra::network::find(&self.database, &arriving.node).await {
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

        let recorded = crate::infra::network::save(
            &self.database,
            &Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
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
                // What it agreed to, said by the node that agreed. This
                // is the only way this side can ever know: the decision
                // is a row on that machine, in a database this one has
                // no access to and should not have.
                //
                // An older node sends nothing and is read as both — the
                // terms every join was made under before there were any.
                allows: match &arriving.accepted {
                    Some(names) => crate::domain::values::capability::parse_list(&names.join(",")),
                    None => crate::domain::values::capability::Capability::ALL.to_vec(),
                },
                // What it will present when this node dials it back over
                // the overlay. Kept from the row when a re-join says
                // nothing — an older node must not lose an anchor it
                // already sent, and a re-join is the one call most likely
                // to come from a node mid-upgrade.
                ca_pem: arriving
                    .ca
                    .clone()
                    .filter(|pem| !pem.trim().is_empty())
                    .or_else(|| known.as_ref().and_then(|node| node.ca_pem.clone())),
            },
        )
        .await;
        if let Err(error) = recorded {
            return Ok(unreadable(error));
        }

        let me = match crate::infra::network::me(&self.database).await {
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
        // **What this node offered, written where it is enforced.** The
        // token said what it would do for whoever spent it, and the
        // joining node recorded that as `allows` — but nothing wrote the
        // matching grant here, and `carry_out` reads *this* table. So an
        // authority that offered `edge` refused the first edge errand it
        // was handed, with "this node has not agreed to `edge` for that
        // node", days after the agreement it was refusing.
        //
        // A grant is still read through what this node provides *now*, so
        // turning the capability off withdraws it: the switch is the more
        // recent decision, and this is only the older one being recorded
        // at all.
        if let Err(error) = crate::infra::network::capability::grant(
            &self.database,
            &arriving.node,
            &enrolment.offers,
        )
        .await
        {
            tracing::warn!(%error, node = %arriving.node, "could not record what this node offered");
        }

        if let Err(error) =
            crate::infra::network::tunnel::ensure(&self.database, &self.config).await
        {
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
                // The same source `join` and every report use for their
                // own: the local authority's public half, which is what
                // makes a call to this node verified rather than merely
                // encrypted.
                ca: crate::infra::edge::certs::ca_certificate_pem(&self.database)
                    .await
                    .ok(),
            },
        ))
    }
}

/// Nothing is discovered: a type left out here panics on resolve.
pub fn register(container: &Container, config: crate::infra::config::Config) {
    container.register_instance::<crate::infra::config::Config>(Arc::new(config));
    register_transients!(container, RemoteNodeRestController);
}

pub fn routes(container: &Container) -> Router {
    RemoteNodeRestController::register_routes(container, Router::new())
}
