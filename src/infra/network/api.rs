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

use serde::{Deserialize, Serialize};
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::sqlite::SqliteDatabase;

use super::{enrolment, Kind, NetworkResult, Node};
use crate::infra::clock::now_ms;

/// What a node says about itself when it arrives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arriving {
    /// Its own id, minted when it was installed.
    pub node: String,
    pub name: String,
    /// Its overlay public key, so the authority has everything it needs
    /// to build a tunnel when there is one to build.
    pub public_key: String,
    /// What it agreed the enrolling node may ask of it, out of what the
    /// token asked for.
    ///
    /// It travels because it has to: the decision is a row on the
    /// joining machine, and the node that will be doing the asking has
    /// no way to read it. Absent from an older node, and read as
    /// everything — the terms a join carried before it carried any.
    #[serde(default)]
    pub accepted: Option<Vec<String>>,
    /// Its own certificate authority, in PEM, so this node can dial it
    /// back over the overlay and verify what answers.
    ///
    /// The join is the first moment it can travel and the right one: this
    /// node minted the token, the callback is authenticated by it, so what
    /// arrives is attributable to the node that was enrolled. Refreshed by
    /// every report afterwards.
    #[serde(default)]
    pub ca: Option<String>,
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
    /// **The authority's certificate authority**, without which the
    /// joining node cannot dial it back.
    ///
    /// The relationship was one-way in a way nobody had noticed: the
    /// arriving node sends its own CA and the authority stores it, so the
    /// authority can reach it over the overlay — and the joiner wrote
    /// `None` for the authority's, so `call::to_node` refuses the one node
    /// it should always be able to reach. `doctor` said "it has not
    /// reported since this version" about the machine that had just
    /// enrolled it.
    ///
    /// `serde(default)` so a new node against an older authority still
    /// parses this answer; it then learns the anchor from the roster
    /// instead.
    #[serde(default)]
    pub ca: Option<String>,
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

/// Which copy's log, in the names both nodes already use for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogWanted {
    pub project: String,
    pub service: String,
    pub slot: u32,
    /// The whole kept history rather than the live file — every rotated
    /// generation read as one, which is what somebody looking for what
    /// happened before a restart wants.
    #[serde(default)]
    pub all: bool,
}

/// How much of a log crosses at once.
///
/// The same ceiling the local page reads with, so the two views are the
/// same size — and well under `call::ask_node`'s own bound, which is
/// what stops a far node's answer being this one's memory problem.
pub(crate) const MAX_LOG: usize = 512 * 1024;

/// What a joined node asks its authority to do.
///
/// The kind travels as a string rather than as [`errand::Kind`]: a node
/// older than the instruction has to be able to *read* the message
/// before refusing it, and a strict enum would fail at deserialisation
/// with a message about JSON rather than about versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asked {
    pub kind: String,
    pub payload: serde_json::Value,
}

/// What a node holding somebody else's replicas says about them.
///
/// Sent by the node running them, on the same schedule it collects
/// errands on. The authority placed them and has no other way to know:
/// nothing can reach a private node to ask.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub replicas: Vec<ReplicaState>,
    /// Where the world can dial the reporting node, if anywhere.
    ///
    /// Enrolment records every joining node as private, because it
    /// arrived through a token rather than being dialled and the
    /// enrolling node has no address for it but the overlay one it just
    /// allocated. That was right about reachability and wrong about
    /// what the owner needs: a node it cannot see is public is a node
    /// it can never choose as an edge, so phase 7's whole point — a
    /// name served by any public node on the network — was unreachable
    /// from the one page that decides it.
    ///
    /// It travels on the report rather than at join time so an already
    /// joined node heals itself on its next poll, and so a node that
    /// gains or loses a domain stops being wrong within one interval.
    ///
    /// `None` from an older node, which reads as "no change" rather
    /// than "private" — a report that omits it must not demote a node
    /// the operator can see is public.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// How full that machine is: memory used of total, disk free of total.
    ///
    /// `None` from a node older than this field, which leaves the columns
    /// alone rather than clearing them — the same rule `endpoint`, `allows`
    /// and `ca` follow, and for the same reason: a report that omits a
    /// field must not undo what is known.
    ///
    /// It travels on every report and is **never** part of "something
    /// changed". A reading differs each time by construction, so counting
    /// it would rebuild the route table, rewrite every container's
    /// `/etc/hosts` and wake the certificate loop every fifteen seconds —
    /// the storm `record` was fixed for, arriving through a field nobody
    /// thought of as a decision. `note_reachability` returns `()` and
    /// cannot feed the count, which is what makes that structural rather
    /// than remembered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<super::Usage>,
    /// What it has left to promise: allocatable and committed, memory
    /// and CPU, and how many copies it holds with no ceiling.
    ///
    /// Beside `usage` and answering the other question — a machine with
    /// little in use may have promised every byte it has. `None` from an
    /// older node leaves the columns alone, and travels the same road for
    /// the same reason: `note_reachability` returns `()`, so this cannot
    /// feed the count either. See migration `0051`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<super::Capacity>,
    /// What the reporting node has agreed the *reader* may ask of it.
    ///
    /// Comma-separated, stored on that node's row — see migration
    /// `0026`. The decision is a row on the reporting machine and this
    /// is the only way it can travel; on every report rather than only
    /// at join, so revoking something stops it being offered within one
    /// interval instead of at a next join that never comes.
    ///
    /// `None` from an older node, read as "no change" — the same rule
    /// `endpoint` follows, and for the same reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allows: Option<String>,
    /// The addresses this node holds on its own networks.
    ///
    /// For two private nodes on one LAN: neither has an `endpoint` — that
    /// is set only for a public node — so on the internet they have no
    /// path to each other, and on a LAN they are one hop apart and the
    /// only thing missing was knowing it.
    ///
    /// `None` from a node older than the field, which changes nothing. An
    /// **empty** list is a node saying it has no address worth naming, and
    /// clears what was stored — the same rule `allows` follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan: Option<Vec<String>>,
    /// The reporting node's own certificate authority, in PEM.
    ///
    /// So the reader can dial it back over the overlay and verify what
    /// answers — see `docs/network.md` phase 9, and `call::to_node`. On
    /// every report rather than only at join, so a node enrolled before
    /// this existed becomes dialable on its next poll instead of needing
    /// to join again.
    ///
    /// It is not a secret: a certificate authority's certificate is the
    /// public half, which is the whole point of publishing it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca: Option<String>,
    /// The release the reporting node is running.
    ///
    /// Nothing carried this, so a console looking at its own network could
    /// not say which machine was behind — and asking one to catch up needs
    /// to know that first. Learned like the fields above it, `None` from an
    /// older node leaving the column alone.
    ///
    /// **Never part of "something changed."** Nothing is derived from it,
    /// and `note_reachability` cannot feed the count anyway — the shape
    /// `record` was fixed into, for the storm a per-report field caused
    /// once already.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// How the reporting node's last update went, in its own words.
    ///
    /// Empty means nothing is wrong. An update errand is settled the
    /// moment an install *starts* — it must be, because the install ends
    /// by replacing the process that would have settled it — so this is
    /// the only road a failed download has back to the node that asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_note: Option<String>,
    /// What the reader must present to be listened to when it dials this
    /// node back over the overlay.
    ///
    /// The direction phase 9 opened and left unauthenticated. It travels
    /// here because this is the one channel that is already both
    /// authenticated and repeated: the reporting node knows who it is
    /// talking to, and a node that was enrolled before this existed
    /// learns it on its next poll rather than needing to join again.
    ///
    /// Derived rather than stored — see `network::dial_secret_for` — so
    /// sending it on every report costs a hash and gives the reader no
    /// way to be out of date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dial: Option<String>,
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
    /// What it is using, from the cgroup of the machine running it.
    ///
    /// The owner of a service cannot read another node's cgroups, so this
    /// is the only way the figure can exist there — the same reason the
    /// address and the failure travel. `None` from a node that has not
    /// upgraded, and from a copy that is not running.
    ///
    /// **It is a measurement, not a decision**, and that difference has
    /// teeth: see `record`, which must not treat a new figure as news.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<u64>,
    /// What its volume holds on that machine's disk.
    ///
    /// The figure nothing else can produce here, and the one that matters
    /// most for a database: a ceiling on memory is a decision somebody
    /// made, and a volume grows until the machine is full. A measurement,
    /// with the same rule as the one above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<u64>,
    /// What it is using, in millicores, **already divided**.
    ///
    /// Memory and disk are absolutes and cross as read. A rate is the
    /// difference between two samples, and only the node running the copy
    /// has both — sending its counter would leave the reader dividing by
    /// the gap since the last report, which is fifteen seconds when all is
    /// well and unknown when it is not.
    ///
    /// So the window differs by where the copy is: two seconds for one
    /// here, a report interval for one elsewhere. Same unit, and the page
    /// says which is which.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<u32>,
}

/// A refusal, in the shape the other end knows how to print.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub error: String,
}

/// A body past this is not a node describing itself.
pub(crate) const MAX_BODY: usize = 64 * 1024;

/// What a co-edge asks this node to serve.
///
/// The response is the token's key authorization — public by design: it
/// is fetched over plain HTTP by the authority, which is the whole shape
/// of an HTTP-01 challenge. What must not be public is the *ability to
/// choose one*, which is what the secret guards.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Challenge {
    pub hostname: String,
    pub token: String,
    pub response: String,
}

/// Compare two secrets without leaking where they differ.
///
/// A short-circuiting `==` on a secret is a timing oracle: an attacker
/// who can call this endpoint learns the first wrong byte from how long
/// the answer took. Length is compared first and then every byte, which
/// is what makes the time depend on the length alone.
pub(crate) fn constant_time_eq(held: &[u8], presented: &[u8]) -> bool {
    if held.len() != presented.len() {
        return false;
    }
    held.iter()
        .zip(presented)
        .fold(0u8, |differences, (a, b)| differences | (a ^ b))
        == 0
}

/// The fields that have to be there for any of this to mean anything.
///
/// Checked here rather than trusted, because everything below writes
/// them into a row: an empty id would be a node nothing can name, and a
/// three-kilobyte one would be a table nobody can read.
pub(crate) fn describes_a_node(arriving: &Arriving) -> Result<(), &'static str> {
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
pub(crate) async fn asking(database: &SqliteDatabase, secret: &str) -> Result<String, Response> {
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

/// Which node is asking, by either credential it could be holding.
///
/// Two roads reach this node and each has its own secret, because each
/// runs the other way:
///
/// * A **member** dials its authority's public endpoint holding the
///   secret its join token carried — [`asking`], and every endpoint above
///   this one.
/// * An **authority** dials a member over the overlay holding the secret
///   that member derived for it and sent on a report — see
///   `network::who_dialled`.
///
/// An endpoint that can be reached both ways has to accept both, and it
/// is the same question either way: which node is this. Tried in that
/// order because the first is a single indexed read and the second is a
/// hash per authority.
pub(crate) async fn caller(database: &SqliteDatabase, secret: &str) -> Result<String, Response> {
    if let Ok(node) = asking(database, secret).await {
        return Ok(node);
    }
    match super::who_dialled(database, secret).await {
        Ok(Some(node)) => Ok(node),
        Ok(None) => Err(refuse(
            StatusCode::UNAUTHORIZED,
            "this endpoint needs the secret this node gave you to dial it with",
        )),
        Err(error) => Err(unreadable(error)),
    }
}

/// The secret out of an `Authorization: Bearer` header.
pub(crate) fn bearer(request: &Request) -> Option<&str> {
    let value = request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?
        .trim();
    (!value.is_empty()).then_some(value)
}

pub(crate) fn json<T: Serialize>(status: StatusCode, value: &T) -> Response {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("a serialised body is well-formed")
}

/// Record where the world can dial a node that just reported, and that
/// it was heard from.
///
/// The node's own answer is the only one available: everything this
/// node knows about it arrived through a join token or over the
/// overlay, neither of which carries a public address. Trusting it is
/// bounded — the caller already proved it holds this node's credential,
/// and the worst a lying one achieves is being offered as an edge for a
/// name that then fails to resolve, which is visible immediately.
///
/// The endpoint is never dialled for an errand. Errands are collected,
/// not delivered, so nothing here reaches outward at all; this is read
/// by `may_be_edge` and shown on the nodes page, and that is all.
///
/// `None` means the report said nothing, which is an older node — left
/// alone rather than demoted, because a report that omits a field must
/// not undo what the operator can see is true.
pub(crate) async fn note_reachability(
    database: &SqliteDatabase,
    node_id: &str,
    report: &Report,
) -> NetworkResult<()> {
    let (endpoint, allows, ca, usage, capacity) = (
        report.endpoint.as_deref(),
        report.allows.as_deref(),
        report.ca.as_deref(),
        report.usage,
        report.capacity,
    );
    let Some(node) = super::find(database, node_id).await? else {
        return Ok(());
    };

    // An endpoint it has stopped having *is* news: a node whose domain
    // was taken away is no longer somewhere to send the internet, and
    // leaving it in the picker would offer a name that resolves to
    // nothing. An empty string is that, said by a node that has one
    // field for both answers.
    let endpoint = match endpoint {
        Some(endpoint) if endpoint.trim().is_empty() => None,
        // **And a private address is not an endpoint.** The rule
        // `ensure_self` follows for this node's own row, applied to what
        // arrives: a node named after its LAN address used to report one,
        // and storing it made that node *public* here — dialled at an
        // address this machine's network resolves to something else
        // entirely, and offered as an edge for names it can never answer
        // for. The sender no longer sends one; this is what heals a row
        // that already has one, and what stops an older node writing a
        // new one.
        Some(endpoint) if is_private_endpoint(endpoint) => {
            tracing::debug!(node = %node_id, %endpoint, "not an address the world can dial");
            None
        }
        Some(endpoint) => Some(endpoint.to_string()),
        None => node.endpoint.clone(),
    };

    // What it has agreed this node may ask of it. Its own answer, and
    // the only one available — the grant is a row on that machine. An
    // absent field is an older node and changes nothing; an **empty**
    // one is a node that has revoked everything, which is a thing it is
    // entitled to do and this side has to hear.
    let allows = match allows {
        Some(list) => crate::domain::values::capability::parse_list(list),
        None => node.allows.clone(),
    };

    // The authority it presented, so a call *to* it over the overlay can
    // be verified and not merely encrypted. Refreshed here and not only at
    // join for the reason the two above are: the answer has to travel, and
    // a node already joined would otherwise stay undialable until somebody
    // joined it again. `None` is an older node — left alone, never cleared,
    // because a report that omits a field must not take away what arrived
    // when the node was enrolled.
    let ca_pem = match ca {
        Some(pem) if !pem.trim().is_empty() => Some(pem.to_string()),
        _ => node.ca_pem.clone(),
    };

    super::save(
        database,
        &Node {
            allows,
            ca_pem,
            // Absent is an older node, so the last reading stays. It is
            // stale rather than wrong, and the page says when it was
            // heard from.
            usage: usage.or(node.usage),
            // And the same for what it has left to promise. Stale is the
            // right failure here too: a selector reads `last_seen_at`
            // beside this and declines to *choose* a machine it has not
            // heard from, which is a different question from refusing
            // one.
            capacity: capacity.or(node.capacity),
            // Derived from the endpoint rather than reported, the same
            // way this node decides its own: a kind somebody can send
            // is a kind somebody can send wrongly, and an unrecognised
            // node must never be offered as somewhere to send traffic.
            kind: match endpoint.is_some() {
                true => Kind::Public,
                false => Kind::Private,
            },
            endpoint,
            last_seen_at: Some(now_ms()),
            // What it is running, and how its last update went. Both are
            // facts about that machine held by that machine, which is what
            // makes the report the only road they can travel. Absent is an
            // older node and changes nothing; an empty note is a node
            // saying nothing is wrong, and clears what was there.
            version: report.version.clone().or(node.version.clone()),
            update_note: report
                .update_note
                .clone()
                .or_else(|| node.update_note.clone()),
            ..node
        },
    )
    .await
}

/// Write one reported state onto the row this node placed.
///
/// `false` when there is nothing here it matches — a service renamed
/// since, or a replica this node has already moved. Not an error: the
/// other end is describing its own machine truthfully, and it cannot
/// know what has been decided here since it was told.
///
/// And `false` when the report says what the row already said, which is
/// almost every report. The caller reads `true` as "something moved" and
/// rebuilds derived state from it, so the answer has to be about the
/// change rather than about the write having succeeded.
pub(crate) async fn record(
    database: &SqliteDatabase,
    node: &str,
    state: &ReplicaState,
) -> Result<bool, super::NetworkError> {
    let Some(project) = crate::infra::repositories::projects::all(database)
        .await
        .map_err(|error| super::NetworkError::Refused(error.to_string()))?
        .into_iter()
        .find(|project| project.name == state.project)
    else {
        return Ok(false);
    };
    let found = crate::infra::repositories::services::in_project(
        database,
        &project.id,
        crate::domain::values::slug::Slug::of(&state.service).as_str(),
    )
    .await
    .map_err(|error| super::NetworkError::Refused(error.to_string()))?;
    let Some(service) = found else {
        return Ok(false);
    };

    let replica = crate::infra::repositories::replicas::in_slot(database, &service.id, state.slot)
        .await
        .map_err(|error| super::NetworkError::Refused(error.to_string()))?;
    // Only what this node placed *there*. A row that has since been
    // brought home, or moved to a third node, is not this caller's to
    // describe any more.
    let Some(replica) = replica.filter(|replica| replica.node_id.as_deref() == Some(node)) else {
        return Ok(false);
    };

    // Whether this report says anything the row does not already.
    //
    // The writes below are idempotent, so doing them again was harmless
    // and the *answer* was not: `true` means "something moved", and what
    // the caller does with it is rebuild the route table, rewrite every
    // local container's `/etc/hosts` and wake the certificate loop. A
    // node reports every fifteen seconds and almost always repeats
    // itself, so an unchanged report was that work for ever, on a node
    // where nothing had happened — measured on the Ubuntu test node,
    // where it also kept the certificate loop from ever reaching its
    // twelve-hour wait and buried the journal.
    let changed = match state.evicted {
        // Recorded once. `evict` already refuses a row that carries the
        // timestamp, and a node goes on reporting a copy it evicted for
        // as long as the row is there.
        true => replica.evicted_at.is_none(),
        false => {
            replica.address.as_deref() != state.address.as_deref()
                || replica.overlay_port != state.overlay_port
                || replica.last_error.as_deref() != state.error.as_deref()
        }
    };

    // Written on every report, and **never** part of `changed`.
    //
    // Memory is a live figure: it differs on every reading, so counting it
    // as movement would make every report say "something moved" — and the
    // caller answers that by rebuilding the route table, rewriting every
    // container's `/etc/hosts` and waking the certificate loop. That is
    // exactly the storm this function was fixed for earlier today, and it
    // would come back through a field nobody thought of as a decision.
    if !state.evicted && replica.memory_bytes != state.memory {
        crate::infra::repositories::replicas::set_memory(database, &replica.id, state.memory)
            .await
            .map_err(|error| super::NetworkError::Refused(error.to_string()))?;
    }
    if !state.evicted && replica.disk_bytes != state.disk {
        crate::infra::repositories::replicas::set_disk(database, &replica.id, state.disk)
            .await
            .map_err(|error| super::NetworkError::Refused(error.to_string()))?;
    }
    // Kept when a report carries none, rather than cleared. The first
    // report after that node restarted has no rate yet — it has one
    // sample — and blanking the column for it would read as "it stopped
    // using any" rather than "nobody has measured since".
    if !state.evicted && state.cpu.is_some() && replica.cpu_millicores != state.cpu {
        crate::infra::repositories::replicas::set_cpu(database, &replica.id, state.cpu)
            .await
            .map_err(|error| super::NetworkError::Refused(error.to_string()))?;
    }

    if !changed {
        return Ok(false);
    }

    let write = async {
        if state.evicted {
            crate::infra::repositories::replicas::evict(database, &replica.id).await?;
            return Ok::<(), crate::domain::errors::PlatformError>(());
        }
        crate::infra::repositories::replicas::set_address(
            database,
            &replica.id,
            state.address.as_deref(),
        )
        .await?;
        crate::infra::repositories::replicas::set_overlay_port(
            database,
            &replica.id,
            state.overlay_port,
        )
        .await?;
        crate::infra::repositories::replicas::set_last_error(
            database,
            &replica.id,
            state.error.as_deref(),
        )
        .await
    };
    write
        .await
        .map_err(|error| super::NetworkError::Refused(error.to_string()))?;
    Ok(true)
}

/// The services this node owns that `node` is holding a copy of without
/// this node claiming one there.
///
/// **Only where the service is identifiable and ours.** A project or a
/// service this node cannot find is not evidence of an orphan — it is
/// evidence that something was renamed, and concluding "drop it" from a
/// name lookup that missed would take down a copy that is doing its job. A
/// service that arrived from elsewhere is somebody else's to place, so its
/// placement is not this node's to correct.
///
/// What is left is the one case that is unambiguous: this node owns the
/// service, and the slot that node reports is either gone or placed
/// somewhere else.
pub(crate) async fn unclaimed(
    database: &SqliteDatabase,
    node: &str,
    reported: &[ReplicaState],
) -> Vec<crate::domain::entities::service::Service> {
    let projects = crate::infra::repositories::projects::all(database)
        .await
        .unwrap_or_default();
    let mut services: Vec<crate::domain::entities::service::Service> = Vec::new();

    for state in reported {
        if state.evicted {
            continue;
        }
        let Some(project) = projects
            .iter()
            .find(|project| project.name == state.project)
        else {
            continue;
        };
        let found = crate::infra::repositories::services::in_project(
            database,
            &project.id,
            crate::domain::values::slug::Slug::of(&state.service).as_str(),
        )
        .await
        .ok()
        .flatten();
        let Some(service) = found.filter(|service| service.is_ours()) else {
            continue;
        };
        if services.iter().any(|held| held.id == service.id) {
            continue;
        }

        let claimed =
            crate::infra::repositories::replicas::in_slot(database, &service.id, state.slot)
                .await
                .ok()
                .flatten()
                .is_some_and(|replica| {
                    replica.node_id.as_deref() == Some(node) && !replica.evicted()
                });
        if !claimed {
            services.push(service);
        }
    }
    services
}

/// A storage failure, in the same shape as every other answer here.
///
/// The crate's own error body would be a different shape, and the node
/// on the other end reads this one — a refusal it cannot parse is a
/// status code and nothing else. 503 because the right move from over
/// there is to try the same token again.
pub(crate) fn unreadable(error: super::NetworkError) -> Response {
    tracing::error!(%error, "a node tried to join and this one could not read its own records");
    refuse(
        StatusCode::SERVICE_UNAVAILABLE,
        "this node could not read its own records; try again",
    )
}

/// Whether `host:port` names an address only its own network can reach.
///
/// The host half of it, through the same judgement `ensure_self` makes
/// about a domain — one definition, because a node that will not
/// *advertise* a private address must not have one *stored* for it
/// either, and the two answers disagreeing is how the test pair ended up
/// with a public row for a private machine.
fn is_private_endpoint(endpoint: &str) -> bool {
    let host = endpoint.rsplit_once(':').map_or(endpoint, |(host, _)| host);
    super::is_private_address(host)
}

/// The same answer for the other half of the node's storage.
///
/// A separate function rather than a generic one: the two error types
/// come from different modules and neither is this one's to convert,
/// which is the same reason `NetworkError` exists at all.
pub(crate) fn unreadable_platform(error: crate::domain::errors::PlatformError) -> Response {
    tracing::error!(%error, "this node could not read its own rows for a caller");
    refuse(
        StatusCode::SERVICE_UNAVAILABLE,
        "this node could not read its own records; try again",
    )
}

pub(crate) fn refuse(status: StatusCode, reason: &str) -> Response {
    json(
        status,
        &Refusal {
            error: reason.to_string(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::Config;
    use crate::infra::network::errand;
    use std::sync::Arc;
    use wabot::prelude::*;
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
        let database = Arc::new(crate::infra::db::open_in_memory().await.expect("open"));
        let admin = crate::infra::network::tests::admin(&database).await;
        crate::infra::node::settings::set_domain(&database, Some("hub.example"))
            .await
            .expect("domain");
        let me = super::super::ensure_self(&database, &Config::default())
            .await
            .expect("seeded");

        let address = super::super::overlay::allocate(&database)
            .await
            .expect("allocate");
        let (_, secret) = enrolment::create(
            &database,
            "alpine",
            &address,
            &admin,
            &crate::domain::values::capability::Capability::ALL,
            &crate::domain::values::capability::Capability::ALL,
        )
        .await
        .expect("minted");

        let container = Container::new();
        container.register_instance::<SqliteDatabase>(database.clone());
        // The controller recomputes routes after a report, so it wants
        // one. Without containerd it can only ever read rows, which is
        // all these tests ask of it.
        container.register_instance::<crate::infra::deploy::Deployer>(Arc::new(
            crate::infra::deploy::Deployer::new(database.clone(), &Config::default()),
        ));
        // The controller holds one so an authority can ring it. Nothing in
        // these tests answers it: a ring with no loop behind it is a
        // notification nobody is waiting on, which is exactly what it is
        // on a node whose collector is between passes.
        container.register_instance::<super::super::collect::Doorbell>(Arc::new(
            super::super::collect::Doorbell::default(),
        ));
        crate::web::remote_node_rest_controller::register(&container, Config::default());

        Authority {
            harness: RestHarness::new(crate::web::remote_node_rest_controller::routes(&container)),
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

    /// A log belongs to whoever placed the copy, and to nobody else.
    ///
    /// One local read decides it — the service's origin — because that is
    /// the only thing that knows whose the copy is. Not a capability of
    /// its own: reading what your own container wrote comes with having
    /// been allowed to run it, the way the registry pull does.
    #[tokio::test]
    async fn only_the_node_that_placed_a_replica_may_read_its_log() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        // A service of this node's own, which the joined node has no
        // claim on whatever it presents.
        let project = crate::infra::repositories::projects::create(&authority.database, "mine")
            .await
            .expect("project");
        crate::infra::repositories::services::create(
            &authority.database,
            &project.id,
            "web",
            "docker.io/library/nginx:1",
            &[],
        )
        .await
        .expect("service");

        let asked = serde_json::json!({
            "project": "mine", "service": "web", "slot": 1u32,
        });
        let refused = authority
            .harness
            .post("/api/network/replica-log")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&asked)
            .send()
            .await;
        refused.assert_status(StatusCode::FORBIDDEN);
        assert!(
            refused.body.contains("not placed here by you"),
            "the refusal says which of the two things it is: {}",
            refused.body
        );

        // And a secret nobody was given reaches nothing at all.
        let stranger = authority
            .harness
            .post("/api/network/replica-log")
            .header("authorization", "Bearer made-up")
            .json(&asked)
            .send()
            .await;
        stranger.assert_status(StatusCode::UNAUTHORIZED);
    }

    /// A node that stopped being public says so, and is believed.
    ///
    /// **The distinction the protocol always had and nothing ever used.**
    /// An absent `endpoint` means "an older node cannot say" and keeps
    /// what is stored; an empty one means "I have none". Nothing sent the
    /// empty one, so a node that stopped being public never told anybody —
    /// measured on the test pair, where the public node held the private
    /// one as `192.168.1.55:443` and therefore as **public**: it pointed
    /// its tunnel at an address on somebody else's network at every start,
    /// and would have offered that node as an edge for a name it can never
    /// answer for.
    ///
    /// And a private address is refused outright, which is what heals a
    /// row that already holds one.
    #[tokio::test]
    async fn a_node_that_stopped_being_public_stops_being_dialled() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        let report = |body: serde_json::Value| {
            let harness = &authority.harness;
            let secret = authority.secret.clone();
            async move {
                harness
                    .post("/api/network/report")
                    .header("authorization", format!("Bearer {secret}"))
                    .json(&body)
                    .send()
                    .await
                    .assert_ok();
            }
        };
        let stored = || async {
            super::super::find(&authority.database, "nd-joining001")
                .await
                .expect("query")
                .expect("joined")
        };

        report(serde_json::json!({ "replicas": [], "endpoint": "elsewhere.example:443" })).await;
        assert_eq!(
            stored().await.endpoint.as_deref(),
            Some("elsewhere.example:443")
        );
        assert_eq!(stored().await.kind, Kind::Public);

        // An address only its own network can reach is not an endpoint,
        // whoever says it is.
        report(serde_json::json!({ "replicas": [], "endpoint": "192.168.1.55:443" })).await;
        assert_eq!(
            stored().await.endpoint,
            None,
            "a private address is not somewhere to dial"
        );
        assert_eq!(stored().await.kind, Kind::Private);

        report(serde_json::json!({ "replicas": [], "endpoint": "elsewhere.example:443" })).await;
        assert_eq!(stored().await.kind, Kind::Public, "and it can come back");

        // The empty string: "I have none", which has to be heard.
        report(serde_json::json!({ "replicas": [], "endpoint": "" })).await;
        assert_eq!(stored().await.endpoint, None);
        assert_eq!(stored().await.kind, Kind::Private);

        // An absent field is an older node, and takes nothing away.
        report(serde_json::json!({ "replicas": [], "endpoint": "elsewhere.example:443" })).await;
        report(serde_json::json!({ "replicas": [] })).await;
        assert_eq!(
            stored().await.endpoint.as_deref(),
            Some("elsewhere.example:443"),
            "a report that cannot say must not undo what is known"
        );
    }

    /// A report that omits a field must not take away what is known.
    ///
    /// The rule every learned column here follows — `endpoint`, `allows`,
    /// `ca` — and the version is one of them: a node reporting from an
    /// older binary says nothing about what it is running, and "nothing"
    /// is not "unknown".
    #[tokio::test]
    async fn a_report_without_a_version_leaves_the_column_alone() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        let send = |body: serde_json::Value| {
            let harness = &authority.harness;
            let secret = authority.secret.clone();
            async move {
                harness
                    .post("/api/network/report")
                    .header("authorization", format!("Bearer {secret}"))
                    .json(&body)
                    .send()
                    .await
                    .assert_ok();
            }
        };

        send(serde_json::json!({ "replicas": [], "version": "0.6.6" })).await;
        let after = super::super::find(&authority.database, "nd-joining001")
            .await
            .expect("query")
            .expect("joined");
        assert_eq!(after.version.as_deref(), Some("0.6.6"));

        send(serde_json::json!({ "replicas": [] })).await;
        let older = super::super::find(&authority.database, "nd-joining001")
            .await
            .expect("query")
            .expect("joined");
        assert_eq!(
            older.version.as_deref(),
            Some("0.6.6"),
            "a report that says nothing about the version is not a node with none"
        );
    }

    /// An errand this node has not agreed to is refused **in the answer**.
    ///
    /// **Found on the real network, and it is why the fix mattered.** The
    /// errand was accepted here and then refused by `carry_out` for a
    /// grant that had never been written — and the refusal stayed in a row
    /// on this side. The asker's own copy said "delivered", because
    /// `hand_over` settles on a 200, so fixing the grant changed nothing:
    /// the instruction was identical, so it was never sent again.
    ///
    /// Consent is checkable at the door with one local read, so it is
    /// checked there. Obeying stays deferred.
    #[tokio::test]
    async fn an_errand_this_node_has_not_agreed_to_is_refused_in_the_answer() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();
        // Whatever the enrolment offered, take it away: this is about the
        // answer, not about what was agreed.
        super::super::capability::grant(&authority.database, "nd-joining001", &[])
            .await
            .expect("nothing granted");

        let serving = serde_json::json!({
            "kind": "edge",
            "payload": { "hostname": "shop.example", "upstreams": ["10.42.0.2:30000"] },
        });
        let refused = authority
            .harness
            .post("/api/network/asked")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serving)
            .send()
            .await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN);
        assert!(
            errand::all(&authority.database)
                .await
                .expect("query")
                .iter()
                .all(|record| record.kind != errand::Kind::Edge),
            "a refused errand is not stored to be refused again later"
        );

        // **A withdrawal needs no permission.** Consent is for taking work
        // on, not for putting it down: a node that revoked a grant is
        // often revoking it because something changed, and refusing the
        // message that cleans up leaves a claim and a route behind.
        let stopping = serde_json::json!({
            "kind": "edge",
            "payload": { "hostname": "shop.example", "upstreams": [] },
        });
        let taken = authority
            .harness
            .post("/api/network/asked")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&stopping)
            .send()
            .await;
        assert_eq!(taken.status, StatusCode::ACCEPTED, "{:?}", taken.status);
    }

    /// The census needs the credential the caller enrolled with.
    ///
    /// It says who else this node enrolled — to a node it enrolled. Same
    /// guard as the errand fetch, so there is no new trust to reason
    /// about, and a stranger gets the same refusal.
    #[tokio::test]
    async fn a_roster_needs_the_secret_the_caller_enrolled_with() {
        let authority = authority().await;

        let bare = authority.harness.get("/api/network/roster").send().await;
        assert_eq!(bare.status, StatusCode::UNAUTHORIZED);

        let stranger = authority
            .harness
            .get("/api/network/roster")
            .header("authorization", "Bearer not-a-secret-anybody-gave-out")
            .send()
            .await;
        assert_eq!(stranger.status, StatusCode::UNAUTHORIZED);

        // A real authority has a key and an address of its own — that is
        // what `install` leaves behind — and without both it is not a peer
        // anybody could configure, so the census leaves it out. The
        // harness stops one step short of that.
        super::super::ensure_hub(&authority.database, &Config::default())
            .await
            .expect("a hub");

        // A node that joined, asking with what it joined with.
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();
        let response = authority
            .harness
            .get("/api/network/roster")
            .header("authorization", format!("Bearer {}", authority.secret))
            .send()
            .await;
        response.assert_ok();

        // And what came back describes the authority itself, which is the
        // member a joiner most needs: its anchor is what makes a call back
        // verified rather than merely encrypted.
        let members = response.value()["members"]
            .as_array()
            .expect("members")
            .clone();
        assert!(
            members
                .iter()
                .any(|member| member["node"] == authority.me && member["ca_pem"].is_string()),
            "{members:#?}"
        );
    }

    /// The join answer carries the authority's own anchor.
    ///
    /// **The relationship was one-way and nobody had noticed.** The
    /// arriving node sends its CA and the authority stores it, so the
    /// authority can dial it over the overlay; the joiner wrote `None`
    /// for the authority's, so `call::to_node` — which verifies rather
    /// than merely encrypting — refused the one node it should always be
    /// able to reach. `doctor` reported it as "it has not reported since
    /// this version" about the machine that had just enrolled it.
    #[tokio::test]
    async fn a_join_answer_carries_the_authoritys_anchor() {
        let authority = authority().await;

        let response = authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await;
        response.assert_ok();

        let answer = response.value();
        let ca = answer["ca"].as_str().expect("an anchor in the answer");
        // Not any string: what `call::to_node` will hand to a root store,
        // so it has to parse as a certificate.
        assert!(
            !crate::infra::edge::certs::pem_certificates(ca)
                .expect("it parses")
                .is_empty(),
            "the anchor holds no certificate"
        );
    }

    /// A co-edge's challenge is served, and only with the secret for
    /// that name.
    ///
    /// **This endpoint has to be authenticated**, unlike every other
    /// thing a node can be asked over the overlay: somebody who could
    /// plant a response would get a certificate for a name this node
    /// serves. The secret is per hostname, minted by the node that owns
    /// the service, and it is the only thing that makes the write
    /// possible.
    #[tokio::test]
    async fn a_challenge_is_taken_only_from_a_node_that_holds_the_secret() {
        let authority = authority().await;
        let project = crate::infra::repositories::projects::create(&authority.database, "shop")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &authority.database,
            &project.id,
            "web",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("service");
        // Being an edge for the name is what mints the secret here.
        crate::infra::repositories::edges::set(
            &authority.database,
            &service.id,
            "shop.example.com",
            &["nd-other".to_string()],
        )
        .await
        .expect("edges");
        let secret =
            crate::infra::repositories::edges::secret_for(&authority.database, "shop.example.com")
                .await
                .expect("query")
                .expect("minted");

        let asking = serde_json::json!({
            "hostname": "shop.example.com",
            "token": "tok-from-the-other-node",
            "response": "tok-from-the-other-node.thumbprint",
        });

        // No secret at all, and the wrong one: both refused, and nothing
        // stored either time.
        for presented in ["", "not-the-secret"] {
            let response = authority
                .harness
                .post("/api/network/challenge")
                .header("authorization", format!("Bearer {presented}"))
                .json(&asking)
                .send()
                .await;
            assert_eq!(response.status, StatusCode::FORBIDDEN, "{presented:?}");
        }
        assert!(
            crate::infra::edge::acme::challenge_response(
                &authority.database,
                "tok-from-the-other-node"
            )
            .await
            .expect("read")
            .is_none(),
            "a refused publish stores nothing"
        );

        // And with it, the node answers for a token it did not order.
        let response = authority
            .harness
            .post("/api/network/challenge")
            .header("authorization", format!("Bearer {secret}"))
            .json(&asking)
            .send()
            .await;
        response.assert_ok();
        assert_eq!(
            crate::infra::edge::acme::challenge_response(
                &authority.database,
                "tok-from-the-other-node"
            )
            .await
            .expect("read")
            .as_deref(),
            Some("tok-from-the-other-node.thumbprint")
        );

        // A name this node is not an edge of has no secret, so there is
        // nothing that could authorize one.
        let elsewhere = authority
            .harness
            .post("/api/network/challenge")
            .header("authorization", format!("Bearer {secret}"))
            .json(&serde_json::json!({
                "hostname": "somebody-elses.example.com",
                "token": "tok-2",
                "response": "tok-2.thumbprint",
            }))
            .send()
            .await;
        assert_eq!(elsewhere.status, StatusCode::FORBIDDEN);
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
    /// The doorbell asks the caller who it is, rather than believing the
    /// path.
    ///
    /// Phase 9 shipped it unauthenticated and said so, because no
    /// credential existed for a call *into* a node. One does now, and the
    /// worst a stranger could do was always small — which is why the
    /// endpoint could ship at all, not a reason to leave it.
    #[tokio::test]
    async fn only_a_node_this_one_knows_can_ring_it() {
        let authority = authority().await;

        // Nothing presented, and a path that claims to be somebody.
        authority
            .harness
            .post("/api/network/wake/nd-stranger")
            .send()
            .await
            .assert_status(StatusCode::UNAUTHORIZED);

        // A secret nobody was given.
        authority
            .harness
            .post("/api/network/wake/nd-stranger")
            .header("authorization", "Bearer made-up")
            .send()
            .await
            .assert_status(StatusCode::UNAUTHORIZED);

        // And the node that enrolled through this one, which is the case
        // the old check declined: a member handing an errand *up* is
        // ringing about something this node is the one to do.
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();
        let response = authority
            .harness
            .post("/api/network/wake/nd-joining001")
            .header("authorization", format!("Bearer {}", authority.secret))
            .send()
            .await;
        response.assert_ok();
        assert_eq!(response.value()["settled"], serde_json::json!(true));
    }

    /// The direction the model did not have: a node this one enrolled
    /// asking *it* to do something. Collection asks a node's
    /// authorities, so an errand addressed to an authority was asked
    /// for by nobody — and being an edge is a row like any other, so a
    /// service owned by a joined node and served by the node that
    /// enrolled it produces one every time.
    ///
    /// Queued, not carried out: this is a request that can time out and
    /// be retried, and obeying an errand writes rows and starts
    /// deployments. What matters here is that it lands, addressed to
    /// this node, with the asker recorded — because what obeying it
    /// decides is asked of *them*.
    #[tokio::test]
    async fn a_node_this_one_enrolled_can_ask_it_to_do_something() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        let response = authority
            .harness
            .post("/api/network/asked")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serde_json::json!({
                "kind": "edge",
                "payload": { "hostname": "api.example.com", "upstreams": ["10.42.0.9:30001"] },
            }))
            .send()
            .await;
        response.assert_status(StatusCode::ACCEPTED);

        let me = super::super::me(&authority.database)
            .await
            .expect("query")
            .expect("there");
        let waiting = errand::waiting_here(&authority.database, &me.id)
            .await
            .expect("query");
        assert_eq!(waiting.len(), 1, "addressed to this node");
        let (order, from) = &waiting[0];
        assert_eq!(order.kind, errand::Kind::Edge);
        assert_eq!(
            from.as_deref(),
            Some("nd-joining001"),
            "and it remembers who asked, which is what obeying it is decided by"
        );

        // A stranger cannot. The bearer is the enrolment secret, so
        // this is the same standing every other endpoint here asks for.
        authority
            .harness
            .post("/api/network/asked")
            .header("authorization", "Bearer not-a-secret")
            .json(&serde_json::json!({ "kind": "edge", "payload": {} }))
            .send()
            .await
            .assert_status(StatusCode::UNAUTHORIZED);

        // And an instruction from a newer node is refused by name,
        // rather than stored and failed later on a machine the asker
        // cannot see.
        authority
            .harness
            .post("/api/network/asked")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serde_json::json!({ "kind": "teleport", "payload": {} }))
            .send()
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }

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

        let after = crate::infra::repositories::replicas::in_slot(&authority.database, &placed, 2)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(after.address.as_deref(), Some("10.42.7.3"));
        assert!(!after.evicted());
    }

    /// Enrolment records every joining node as private, and both nodes
    /// were right: it arrived through a token, and this one has no
    /// address for it but the overlay. It also made phase 7's whole
    /// point unreachable — a node the owner cannot see is public is one
    /// it can never choose as an edge. The node's own report is the
    /// only place that answer exists.
    #[tokio::test]
    async fn a_node_that_reports_an_endpoint_can_be_chosen_as_an_edge() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        let joined = super::super::find(&authority.database, "nd-joining001")
            .await
            .expect("query")
            .expect("joined");
        assert!(
            !joined.may_be_edge(),
            "nothing has said where the world can dial it"
        );

        authority
            .harness
            .post("/api/network/report")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serde_json::json!({
                "replicas": [],
                "endpoint": "alpine.example:443"
            }))
            .send()
            .await
            .assert_ok();

        let after = super::super::find(&authority.database, "nd-joining001")
            .await
            .expect("query")
            .expect("joined");
        assert_eq!(after.kind, Kind::Public);
        assert_eq!(after.endpoint.as_deref(), Some("alpine.example:443"));
        assert!(after.may_be_edge());
        assert!(after.last_seen_at.is_some(), "it was just heard from");
    }

    /// The grant is a row on the *other* machine, so the node doing the
    /// asking can only ever be told. It arrives on the report, which is
    /// what lets a node revoke something and have the other side stop
    /// offering it within one interval — rather than at a next join
    /// that never comes.
    /// A reading cannot be counted as movement, by construction.
    ///
    /// This is the trap this file was already fixed for once. A reading
    /// differs on every report, so if it counted the authority would
    /// rebuild its route table, rewrite every container's `/etc/hosts` and
    /// wake the certificate loop every fifteen seconds for as long as both
    /// nodes were up — 41 rebuilds in ten minutes, measured, when `record`
    /// had that shape.
    ///
    /// Two facts keep it from coming back, and both are read out of the
    /// source because that is where they live. `note_reachability` writes
    /// the reading and returns `()`, which cannot be added to a count; and
    /// `record` — the one function whose answer *is* the count — never
    /// mentions the reading at all. A test on a response body cannot say
    /// either thing: the handler answers `{"settled": true}` and nothing
    /// else, so an assertion about a count passes no matter what. That is
    /// how the first version of this passed.
    #[test]
    fn a_reading_cannot_be_mistaken_for_movement() {
        let source = include_str!("api.rs");

        let signature = source
            .split("async fn note_reachability(")
            .nth(1)
            .expect("the function that writes the reading");
        let signature = signature.split('{').next().expect("its signature");
        assert!(
            signature.contains("NetworkResult<()>"),
            "the writer of the reading answers with something countable: {signature}"
        );

        let counted = source
            .split("async fn record(")
            .nth(1)
            .expect("the function whose answer is the count");
        let counted = counted.split("\nasync fn ").next().expect("its body");
        for field in ["usage", "capacity"] {
            assert!(
                !counted.contains(field),
                "`{field}` is written where the count is decided: {counted}"
            );
        }
    }

    /// How full a node is travels, and a new reading is not a change.
    ///
    /// A node's page had nothing to say about a machine it cannot reach:
    /// the report carried replicas, an endpoint and a list of permissions
    /// and nothing about the machine itself. Asked for by Jorge, so the
    /// network area answers the question somebody opens it with.
    ///
    /// That a reading is **not** movement is asserted separately, by
    /// `a_reading_cannot_be_mistaken_for_movement` — it is a property of
    /// the shape and not of a response body, and the first version of this
    /// test looked for a count the handler does not send, so it passed
    /// against anything.
    #[tokio::test]
    async fn how_full_a_node_is_travels_and_is_not_a_change() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        let report =
            |usage: serde_json::Value| serde_json::json!({ "replicas": [], "usage": usage });
        macro_rules! send {
            ($body:expr) => {
                authority
                    .harness
                    .post("/api/network/report")
                    .header("authorization", format!("Bearer {}", authority.secret))
                    .json(&$body)
                    .send()
                    .await
            };
        }

        send!(report(serde_json::json!({
            "memory_total": 972u64 * 1024 * 1024,
            "memory_used": 363u64 * 1024 * 1024,
            "disk_total": 24u64 * 1024 * 1024 * 1024,
            "disk_free": 22u64 * 1024 * 1024 * 1024,
        })))
        .assert_ok();

        let after = super::super::find(&authority.database, "nd-joining001")
            .await
            .expect("query")
            .expect("joined");
        let usage = after.usage.expect("the reading arrived");
        assert_eq!(usage.memory_used, 363 * 1024 * 1024);
        assert_eq!(usage.disk_free, 22 * 1024 * 1024 * 1024);

        // **A new reading is not news.** The response says how many
        // replicas moved, and a report whose only difference is the
        // machine being a little fuller must say none — otherwise every
        // report is movement and the authority rebuilds everything on a
        // node where nothing happened.
        send!(report(serde_json::json!({
            "memory_total": 972u64 * 1024 * 1024,
            "memory_used": 501u64 * 1024 * 1024,
            "disk_total": 24u64 * 1024 * 1024 * 1024,
            "disk_free": 21u64 * 1024 * 1024 * 1024,
        })))
        .assert_ok();
        // A fuller machine is still written.
        let after = super::super::find(&authority.database, "nd-joining001")
            .await
            .expect("query")
            .expect("joined");
        assert_eq!(
            after.usage.expect("still there").memory_used,
            501 * 1024 * 1024
        );

        // A report that omits it leaves the last reading alone, the way
        // `endpoint`, `allows` and `ca` are left: an older node must not
        // blank a column by not knowing about it.
        send!(serde_json::json!({ "replicas": [] })).assert_ok();
        let after = super::super::find(&authority.database, "nd-joining001")
            .await
            .expect("query")
            .expect("joined");
        assert_eq!(
            after.usage.expect("kept").memory_used,
            501 * 1024 * 1024,
            "an older node's report cleared the reading"
        );
    }

    /// What a node has left to promise travels, and is not a change
    /// either.
    ///
    /// **The other half of the reading, and the half a placement needs.**
    /// `usage` says how busy a machine is right now; a node with 200 MB
    /// in use may have undertaken every byte it has to services that
    /// happen to be idle, so choosing by the meter chooses a full
    /// machine. Nothing carried what had been *promised* until now, which
    /// is why nothing on either side of a placement could ask whether it
    /// would fit.
    ///
    /// That it is **not** movement is asserted by
    /// `a_reading_cannot_be_mistaken_for_movement`, which names it beside
    /// `usage`: it is a property of the shape rather than of a response
    /// body, and the sibling of this test learned that the hard way by
    /// looking for a count the handler does not send. A commitment moves
    /// rarely, being a decision — but `memory_allocatable` hangs off
    /// `memory_total`, and one number flickering re-earns the storm
    /// `record` was fixed for.
    #[tokio::test]
    async fn what_a_node_has_left_travels() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        macro_rules! send {
            ($body:expr) => {
                authority
                    .harness
                    .post("/api/network/report")
                    .header("authorization", format!("Bearer {}", authority.secret))
                    .json(&$body)
                    .send()
                    .await
            };
        }
        macro_rules! room {
            () => {
                super::super::find(&authority.database, "nd-joining001")
                    .await
                    .expect("query")
                    .expect("joined")
                    .capacity
            };
        }

        assert_eq!(
            room!(),
            None,
            "a node that has not said cannot be read as one with nothing left"
        );

        send!(serde_json::json!({
            "replicas": [],
            "capacity": {
                "memory_allocatable": 716u64 * 1024 * 1024,
                "memory_committed": 512u64 * 1024 * 1024,
                "cpu_allocatable": 750,
                "cpu_committed": 500,
                "unbounded": 1,
            },
        }))
        .assert_ok();

        let room = room!().expect("it arrived");
        assert_eq!(room.memory_allocatable, 716 * 1024 * 1024);
        assert_eq!(room.memory_committed, 512 * 1024 * 1024);
        assert_eq!(room.cpu_allocatable, 750);
        assert_eq!(room.cpu_committed, 500);
        assert_eq!(
            room.unbounded, 1,
            "and the count that says the sum is a floor rather than a total"
        );

        // A report that omits it leaves the last answer alone, the way
        // every other learned field is left: an older node must not blank
        // a column by not knowing about it.
        send!(serde_json::json!({ "replicas": [] })).assert_ok();
        assert_eq!(
            room!().expect("kept").memory_committed,
            512 * 1024 * 1024,
            "an older node's report cleared what it had promised"
        );
    }

    #[tokio::test]
    async fn what_a_node_allows_travels_on_its_report() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        for (reported, expected) in [
            (
                "host,edge",
                vec![
                    crate::domain::values::capability::Capability::Host,
                    crate::domain::values::capability::Capability::Edge,
                ],
            ),
            (
                "edge",
                vec![crate::domain::values::capability::Capability::Edge],
            ),
            ("", Vec::new()),
        ] {
            authority
                .harness
                .post("/api/network/report")
                .header("authorization", format!("Bearer {}", authority.secret))
                .json(&serde_json::json!({ "replicas": [], "allows": reported }))
                .send()
                .await
                .assert_ok();

            let after = super::super::find(&authority.database, "nd-joining001")
                .await
                .expect("query")
                .expect("joined");
            assert_eq!(after.allows, expected, "after reporting {reported:?}");
        }
    }

    /// A node whose domain was taken away is no longer somewhere to
    /// send the internet, and leaving it in the picker would offer a
    /// name that resolves to nothing.
    #[tokio::test]
    async fn a_node_that_stops_being_reachable_stops_being_an_edge() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        for endpoint in ["alpine.example:443", ""] {
            authority
                .harness
                .post("/api/network/report")
                .header("authorization", format!("Bearer {}", authority.secret))
                .json(&serde_json::json!({ "replicas": [], "endpoint": endpoint }))
                .send()
                .await
                .assert_ok();
        }

        let after = super::super::find(&authority.database, "nd-joining001")
            .await
            .expect("query")
            .expect("joined");
        assert_eq!(after.kind, Kind::Private);
        assert!(!after.may_be_edge());
    }

    /// A report from a node that predates the field says nothing about
    /// reachability, and saying nothing must not undo what the operator
    /// can already see is true.
    #[tokio::test]
    async fn a_report_without_an_endpoint_leaves_one_alone() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();

        authority
            .harness
            .post("/api/network/report")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serde_json::json!({ "replicas": [], "endpoint": "alpine.example:443" }))
            .send()
            .await
            .assert_ok();
        authority
            .harness
            .post("/api/network/report")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serde_json::json!({ "replicas": [] }))
            .send()
            .await
            .assert_ok();

        let after = super::super::find(&authority.database, "nd-joining001")
            .await
            .expect("query")
            .expect("joined");
        assert!(after.may_be_edge(), "an older node must not be demoted");
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

        let after = crate::infra::repositories::replicas::in_slot(&authority.database, &placed, 2)
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

        let replica =
            crate::infra::repositories::replicas::in_slot(&authority.database, &placed, 2)
                .await
                .expect("query")
                .expect("there");
        crate::infra::repositories::replicas::move_to(&authority.database, &replica.id, None)
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

        let after = crate::infra::repositories::replicas::in_slot(&authority.database, &placed, 2)
            .await
            .expect("query")
            .expect("there");
        assert!(!after.evicted(), "a stale report evicted a local replica");
        assert!(after.is_here());
    }

    /// A node reports every fifteen seconds and almost always says what
    /// it said last time. The answer is read as "something moved", and
    /// what the caller does with it is rebuild the route table, rewrite
    /// every local container's `/etc/hosts` and wake the certificate
    /// loop — so saying it to an unchanged report is that work for ever
    /// on a node where nothing is happening.
    ///
    /// This shipped: on the Ubuntu test node the routes were rebuilt
    /// every fifteen seconds for as long as both nodes were up, which
    /// also reset the certificate loop's backoff before it could ever
    /// reach its twelve-hour wait.
    #[tokio::test]
    async fn a_node_saying_the_same_thing_twice_is_not_a_change() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();
        placed_there(&authority, 2).await;

        let reported = |address: &str| ReplicaState {
            project: "shared".into(),
            service: "web".into(),
            slot: 2,
            address: Some(address.to_string()),
            overlay_port: Some(30001),
            error: None,
            evicted: false,
            memory: None,
            disk: None,
            cpu: None,
        };

        let first = record(&authority.database, "nd-joining001", &reported("10.42.2.5"))
            .await
            .expect("recorded");
        assert!(first, "the first report of an address is news");

        let again = record(&authority.database, "nd-joining001", &reported("10.42.2.5"))
            .await
            .expect("recorded");
        assert!(!again, "the same report again is not");

        let moved = record(&authority.database, "nd-joining001", &reported("10.42.2.9"))
            .await
            .expect("recorded");
        assert!(moved, "an address that changed is news again");
    }

    /// A copy the owner does not claim is one nobody will ever stop.
    ///
    /// The instruction that would stop it is queued by whatever made the
    /// change — and cannot be, if the change already happened, if this node
    /// was down when it did, or if the code that should have sent it did
    /// not. All three have happened here, and the last one left a Postgres
    /// running on the Alpine node with a volume and a replication stream
    /// and no row on either side claiming it.
    ///
    /// So the report itself is the trigger: that node says it holds slot 3,
    /// this node's rows say otherwise, and the placement goes out again —
    /// computed from the rows, so it is right whatever the disagreement
    /// was.
    #[tokio::test]
    async fn a_copy_this_node_does_not_claim_is_told_to_go() {
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

        // The row goes away, which is what a placement change leaves —
        // and the far node has not been told.
        let replica =
            crate::infra::repositories::replicas::in_slot(&authority.database, &placed, 2)
                .await
                .expect("query")
                .expect("there");
        crate::infra::repositories::replicas::remove(&authority.database, &replica.id)
            .await
            .expect("removed");

        authority
            .harness
            .post("/api/network/report")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&serde_json::json!({
                "replicas": [
                    { "project": "shared", "service": "web", "slot": 2,
                      "address": "10.42.2.5" }
                ]
            }))
            .send()
            .await
            .assert_ok();

        let told = crate::infra::network::errand::waiting(&authority.database, "nd-joining001")
            .await
            .expect("errands")
            .pop()
            .expect("the node holding it was told nothing");
        assert_eq!(
            told.payload["slots"],
            serde_json::json!([]),
            "which is how it learns the copy is not its to run"
        );
    }

    /// A figure that differs on every reading is not a change.
    ///
    /// Memory travels on the report because the node that placed a copy
    /// cannot read another machine's cgroups — but it is a measurement,
    /// and `true` here means "something moved", which the caller answers
    /// by rebuilding the route table, rewriting every container's
    /// `/etc/hosts` and waking the certificate loop. Counting a new figure
    /// as news would bring back the storm this function was fixed for, on
    /// a field nobody thinks of as a decision.
    ///
    /// It is still *written*: the page reads it, and a stale number is
    /// worse than none.
    #[tokio::test]
    async fn a_new_memory_figure_is_stored_and_is_not_news() {
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

        let reporting = |bytes: u64| ReplicaState {
            project: "shared".into(),
            service: "web".into(),
            slot: 2,
            address: Some("10.42.2.5".into()),
            overlay_port: Some(30001),
            error: None,
            evicted: false,
            memory: Some(bytes),
            disk: None,
            cpu: None,
        };

        assert!(
            record(&authority.database, "nd-joining001", &reporting(1_000))
                .await
                .expect("recorded"),
            "the address is news the first time"
        );
        assert!(
            !record(&authority.database, "nd-joining001", &reporting(2_000))
                .await
                .expect("recorded"),
            "a different figure on its own is not"
        );

        let after = crate::infra::repositories::replicas::in_slot(&authority.database, &placed, 2)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(
            after.memory_bytes,
            Some(2_000),
            "and it is written anyway, or the page shows a stale number"
        );
    }

    /// A rate crosses as an answer, and an absent one leaves what is
    /// there.
    ///
    /// Memory and disk are absolutes and cross as read; CPU is the
    /// difference between two samples, so the node running the copy
    /// divides and sends millicores — a counter would leave this side
    /// dividing by the gap since the last report, which is fifteen seconds
    /// when all is well and unknown when it is not.
    ///
    /// And a report carrying none does not clear it. The first report
    /// after that node restarted has one sample and no rate yet; blanking
    /// the column would read as "it stopped using any" rather than "nobody
    /// has measured since".
    #[tokio::test]
    async fn a_rate_arrives_worked_out_and_an_absent_one_keeps_what_it_had() {
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

        let reporting = |cpu: Option<u32>| ReplicaState {
            project: "shared".into(),
            service: "web".into(),
            slot: 2,
            address: Some("10.42.2.5".into()),
            overlay_port: Some(30001),
            error: None,
            evicted: false,
            memory: Some(1_000),
            disk: None,
            cpu,
        };

        record(&authority.database, "nd-joining001", &reporting(Some(250)))
            .await
            .expect("recorded");
        let after = crate::infra::repositories::replicas::in_slot(&authority.database, &placed, 2)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(
            after.cpu_millicores,
            Some(250),
            "already divided, and stored"
        );

        // A restart on that node: one sample, no rate.
        assert!(
            !record(&authority.database, "nd-joining001", &reporting(None))
                .await
                .expect("recorded"),
            "and it is still not news"
        );
        let after = crate::infra::repositories::replicas::in_slot(&authority.database, &placed, 2)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(
            after.cpu_millicores,
            Some(250),
            "silence is not nought — the column keeps what somebody measured"
        );
    }

    /// An eviction is recorded once, for the same reason: the node that
    /// evicted a copy goes on saying so on every report while the row is
    /// still there.
    #[tokio::test]
    async fn an_eviction_already_recorded_is_not_news_again() {
        let authority = authority().await;
        authority
            .harness
            .post("/api/network/join")
            .header("authorization", format!("Bearer {}", authority.secret))
            .json(&arriving())
            .send()
            .await
            .assert_ok();
        placed_there(&authority, 2).await;

        let evicted = ReplicaState {
            project: "shared".into(),
            service: "web".into(),
            slot: 2,
            address: None,
            overlay_port: None,
            error: None,
            evicted: true,
            memory: None,
            disk: None,
            cpu: None,
        };

        assert!(
            record(&authority.database, "nd-joining001", &evicted)
                .await
                .expect("recorded"),
            "an eviction has to reach the node that placed it"
        );
        assert!(
            !record(&authority.database, "nd-joining001", &evicted)
                .await
                .expect("recorded"),
            "the same eviction on the next report is not a change"
        );
    }

    /// A service with a replica placed on the joined node, and its id.
    async fn placed_there(authority: &Authority, slot: u32) -> String {
        let project = crate::infra::repositories::projects::create(&authority.database, "shared")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &authority.database,
            &project.id,
            "web",
            "hub.example/shared/web@sha256:abc",
            &[],
        )
        .await
        .expect("service");
        crate::infra::repositories::replicas::place(
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
