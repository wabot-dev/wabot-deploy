//! Nodes that know about each other, and who may configure whom.
//!
//! ## Directed authority, not agreement
//!
//! Several nodes sharing a configuration usually means consensus, and
//! consensus is more machinery than this product should carry. It is
//! also the wrong question. A node does not need to agree with its
//! peers about the world; it needs to know **which of them it takes
//! instructions from**.
//!
//! So every relationship is one-way. A node grants authority; the
//! holder sends errands. Two nodes that granted each other nothing
//! cannot affect each other, and nothing has to be reconciled because
//! nothing is shared. See migration `0015`.
//!
//! ## What is here and what is not
//!
//! The model, and enrolment on top of it: a public node mints a token
//! ([`enrolment`], [`token`]), somebody carries it to a private node,
//! and `join` writes the grant and calls back ([`api`]). Nothing is
//! dialled — an errand has no way to travel until the overlay exists,
//! which is the next phase. See `docs/network.md`.
//!
//! ## Private is a consequence, not a category
//!
//! A public node offers to answer for names, which takes an address the
//! internet can dial *and* a node willing to use it. A private node is
//! one that does not offer it — because it cannot, or because it would
//! rather not. See [`capability`].
//!
//! [`Kind`] is still derived rather than trusted, and the property that
//! matters is unchanged: a node can only ever **reduce** what it claims.
//! Offering `Edge` requires the endpoint, so no setting can make a node
//! look reachable when it is not.

pub mod api;
pub mod call;
pub mod capability;
pub mod collect;
pub mod enrolment;
pub mod errand;
pub mod join;
pub mod keys;
pub mod overlay;
pub mod token;
pub mod tunnel;

use serde::{Deserialize, Serialize};
use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::{SqliteDatabase, SqliteResult};

use crate::config::Config;
use crate::platform::now_ms;

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("storage: {0}")]
    Storage(#[from] wabot::sqlite::SqliteError),
    #[error("{0}")]
    Refused(String),
}

pub type NetworkResult<T> = Result<T, NetworkError>;

/// How a failure here reaches an HTTP caller.
///
/// A named type rather than a `From<SqliteError>` because both of those
/// are somebody else's types and the orphan rule says so — the same
/// reason `PlatformError` exists. The judgement is the same too: a
/// refusal is the caller's to fix and says what it was, and the node's
/// own storage failure says nothing beyond having happened.
impl From<NetworkError> for wabot::rest::RestError {
    fn from(error: NetworkError) -> Self {
        match error {
            NetworkError::Refused(message) => wabot::rest::RestError::Client {
                status: 400,
                message,
            },
            other => {
                tracing::error!(error = %other, "network operation failed");
                wabot::rest::RestError::Internal("network operation failed".into())
            }
        }
    }
}

/// What a node can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Reachable from the internet: it can be an edge.
    Public,
    /// Runs containers, reached across the overlay.
    Private,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }

    /// Unknown reads as private: a node we cannot place must not be
    /// offered as somewhere to send the internet.
    pub fn parse(text: &str) -> Self {
        match text {
            "public" => Self::Public,
            _ => Self::Private,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// Where the world dials it, when it offers to be dialled. `None`
    /// for a private node — including one with a perfectly good address
    /// that has turned the capability off, which is a decision and not
    /// a limitation.
    pub endpoint: Option<String>,
    /// Filled when it joins the overlay.
    pub public_key: Option<String>,
    pub overlay_ip: Option<String>,
    pub is_self: bool,
    pub last_seen_at: Option<i64>,
    /// What this node lets **us** ask of it — learned, never decided
    /// here. The decision lives on that machine, in its own
    /// `node_grant`, and it travels on the report it already sends. See
    /// migration `0026`.
    pub allows: Vec<capability::Capability>,
    /// The certificate authority it presented, so a call *to* it over the
    /// overlay can be verified rather than merely encrypted.
    ///
    /// Learned like `allows` and for the same reason: it is a fact about
    /// that machine, held by that machine. `None` is a node that joined
    /// before this existed — dialled the old way, which is not at all. See
    /// migration `0033`.
    pub ca_pem: Option<String>,
}

impl Node {
    /// Whether this node can be asked to serve traffic for a name.
    ///
    /// Both halves are required: an endpoint the world can dial, and a
    /// kind that says so. A row with one and not the other is a node
    /// part-way through being set up, and sending a hostname to it
    /// would publish a name that resolves to nothing.
    ///
    /// For this node's own row the kind already carries its answer to
    /// [`capability::Capability::Edge`], and for any other node it
    /// carries what that node reported about itself — so a node that
    /// declines to be an edge stops being offered as one everywhere,
    /// from the one place it decided.
    pub fn may_be_edge(&self) -> bool {
        self.kind == Kind::Public && self.endpoint.is_some()
    }
}

fn read(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Node> {
    Ok(Node {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: Kind::parse(&row.get::<_, String>(2)?),
        endpoint: row.get(3)?,
        public_key: row.get(4)?,
        overlay_ip: row.get(5)?,
        is_self: row.get::<_, i64>(6)? != 0,
        last_seen_at: row.get(7)?,
        allows: capability::parse_list(&row.get::<_, Option<String>>(8)?.unwrap_or_default()),
        ca_pem: row.get(9)?,
    })
}

const COLUMNS: &str = "\"id\", \"name\", \"kind\", \"endpoint\", \"public_key\", \
                       \"overlay_ip\", \"is_self\", \"last_seen_at\", \"allows\", \
                       \"ca_pem\"";

/// Write, or bring up to date, the row for the node this process is.
///
/// Called by `install` and again by every `serve`, because neither one
/// on its own is enough: an update replaces the binary and restarts
/// without running the installer, and a node installed before this
/// existed has no row at all. Convergent, like every install step — it
/// asks what this node is, not whether it has been asked before.
///
/// The id is minted once and then kept for ever. It is what other nodes
/// call this one, so a new one on every start would be a node that
/// looked like a stranger to everybody it had joined.
/// The name every node has, whatever else it has.
///
/// **Derived from the id, never stored.** No column, nothing on the wire
/// and no two places that can disagree about what a node is called — the
/// id is already minted at install and kept for ever, which is exactly
/// the lifetime this name needs.
///
/// It needs no DNS. A node dialling another resolves this to that node's
/// overlay address itself, which is what makes the whole thing work for a
/// node behind NAT with nothing forwarded: see `call::to_node`, and phase
/// 9 in `docs/network.md` for why the premise that this was impossible was
/// false.
///
/// `.node` is not a public suffix and is not meant to be. A name that
/// could resolve in the world's DNS is a name somebody could be sent to by
/// mistake, and this one is only ever looked up in a map this node wrote.
///
/// **Lower case, because everything that touches a hostname normalises
/// it and they do not all normalise at the same moment.** An id is mixed
/// case; a URI's host is lowercased by the `http` crate, the route table
/// lowercases what it stores, and rustls compares DNS names
/// case-insensitively. Producing the mixed-case form and letting each
/// layer fix it cost two node runs — a resolver that refused the name it
/// was built for, and a route that existed under a spelling the test did
/// not look for. So the name is lower case at birth and nobody downstream
/// has a decision to make.
///
/// Two ids differing only in case would be one name here. That is the
/// collision the id itself would already have to survive, and the join
/// refuses an id this node is using.
pub fn internal_name(id: &str) -> String {
    format!("{}.node", id.to_ascii_lowercase())
}

pub async fn ensure_self(database: &SqliteDatabase, config: &Config) -> NetworkResult<Node> {
    let existing = me(database).await?;
    let domain = crate::node::settings::domain(database, config).await;

    // Derived, not stored: a node answers to a name or it does not, and
    // an operator who cleared the domain has made this node private
    // whatever it used to be.
    let endpoint = domain
        .as_ref()
        .map(|domain| format!("{domain}:{}", config.edge.https_port));
    // Public means *offering* to answer for names, which takes both an
    // address the world can dial and a node willing to use it. An
    // operator who turned the capability off has made this node private
    // as surely as one who cleared the domain — see `capability`.
    let kind = match endpoint.is_some()
        && capability::provides(database, capability::Capability::Edge).await
    {
        true => Kind::Public,
        false => Kind::Private,
    };
    // And the row says so: a node that will not be an edge must not
    // carry an address that invites one, here or on any node it
    // reports to.
    let endpoint = match kind {
        Kind::Public => endpoint,
        _ => None,
    };

    let node = Node {
        id: match &existing {
            Some(node) => node.id.clone(),
            None => format!("nd-{}", wabot::prelude::password::generate(12)),
        },
        name: crate::node::name(domain.as_deref()),
        kind,
        endpoint,
        // Carried forward rather than asked for: this reads the key, it
        // does not mint one. A node that never enrols anybody and never
        // joins anything has no use for a key pair, and generating one
        // on every install would be the install deciding otherwise.
        public_key: keys::public_key(database).await,
        overlay_ip: existing.as_ref().and_then(|node| node.overlay_ip.clone()),
        is_self: true,
        // "When this node last heard from it" is not a question about
        // itself, and answering it would put a heartbeat on the page
        // that only ever says now.
        last_seen_at: None,
        // A node needs no permission from itself. What it will do for
        // anybody is `capability::provides`, and the selectors read
        // that for the self row rather than this.
        allows: Vec::new(),
        ca_pem: None,
    };

    save(database, &node).await?;
    Ok(node)
}

/// Everything this node needs before it can enrol anybody.
///
/// A key pair, a row of its own, and an address on the overlay it is
/// about to be the hub of. Done here rather than at install because a
/// node that never enrols anybody and never joins anything needs none
/// of it — and an address allocated at install would be a fact about an
/// overlay that does not exist.
pub async fn ensure_hub(database: &SqliteDatabase, config: &Config) -> NetworkResult<Node> {
    // Before the row, so the public key lands in it rather than on the
    // next start.
    keys::ensure(database).await?;
    let me = ensure_self(database, config).await?;
    if me.overlay_ip.is_some() {
        return Ok(me);
    }

    let me = Node {
        overlay_ip: Some(overlay::allocate(database).await?),
        ..me
    };
    save(database, &me).await?;
    Ok(me)
}

pub async fn all(database: &SqliteDatabase) -> NetworkResult<Vec<Node>> {
    Ok(database
        .read(|connection| {
            // This node first, then by name. Alphabetical alone put the
            // node somebody is looking at second on a list of two, and
            // would bury it on a list of twenty — it is the one row
            // every other row is being compared against.
            let mut statement = connection.prepare(&format!(
                "SELECT {COLUMNS} FROM node ORDER BY \"is_self\" DESC, \"name\""
            ))?;
            let nodes: wabot::sqlite::rusqlite::Result<Vec<Node>> =
                statement.query_map([], read)?.collect();
            nodes
        })
        .await?)
}

pub async fn find(database: &SqliteDatabase, id: &str) -> NetworkResult<Option<Node>> {
    let id = id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM node WHERE \"id\" = ?1"),
                    [id],
                    read,
                )
                .optional()
        })
        .await?)
}

/// The node this process is.
pub async fn me(database: &SqliteDatabase) -> NetworkResult<Option<Node>> {
    Ok(database
        .read(|connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM node WHERE \"is_self\" = 1"),
                    [],
                    read,
                )
                .optional()
        })
        .await?)
}

/// Write a node, or update what is known about one.
///
/// Convergent, like every install step: it asks about the node, not
/// about whether it has been seen before.
pub async fn save(database: &SqliteDatabase, node: &Node) -> NetworkResult<()> {
    let node = node.clone();
    Ok(database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO node \
                   (\"id\", \"name\", \"kind\", \"endpoint\", \"public_key\", \"overlay_ip\", \
                    \"is_self\", \"joined_at\", \"last_seen_at\", \"allows\", \"ca_pem\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
                 ON CONFLICT (\"id\") DO UPDATE SET \
                   \"name\" = excluded.\"name\", \
                   \"kind\" = excluded.\"kind\", \
                   \"endpoint\" = excluded.\"endpoint\", \
                   \"public_key\" = excluded.\"public_key\", \
                   \"overlay_ip\" = excluded.\"overlay_ip\", \
                   \"last_seen_at\" = excluded.\"last_seen_at\", \
                   \"allows\" = excluded.\"allows\", \
                   \"ca_pem\" = excluded.\"ca_pem\"",
                (
                    node.id,
                    node.name,
                    node.kind.as_str(),
                    node.endpoint,
                    node.public_key,
                    node.overlay_ip,
                    i64::from(node.is_self),
                    now_ms(),
                    node.last_seen_at,
                    capability::to_list(&node.allows),
                    node.ca_pem,
                ),
            )?;
            Ok(())
        })
        .await?)
}

/// Stop knowing about a node.
///
/// One direction only, and it says so on the page: the other node still
/// holds this one as an authority until somebody revokes it there. That
/// is the model working rather than a gap in it — a grant is the
/// granting node's to withdraw, and nothing here can reach in and do it
/// for them.
pub async fn forget(database: &SqliteDatabase, id: &str) -> NetworkResult<()> {
    let id = id.to_string();
    Ok(database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM node WHERE \"id\" = ?1 AND \"is_self\" = 0",
                [id],
            )?;
            Ok(())
        })
        .await?)
}

// ---------- who may configure this node ---------------------------------

/// A grant, as the nodes page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority {
    pub node_id: String,
    pub granted_at: i64,
    pub revoked_at: Option<i64>,
}

impl Authority {
    pub fn live(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// Every node this one has ever taken instructions from.
///
/// Revoked ones included, because "this used to be allowed" is the
/// thing somebody comes to this page to check.
pub async fn authorities(database: &SqliteDatabase) -> NetworkResult<Vec<Authority>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"node_id\", \"granted_at\", \"revoked_at\" \
                     FROM authority ORDER BY \"granted_at\" DESC",
                )?
                .query_map([], |row| {
                    Ok(Authority {
                        node_id: row.get(0)?,
                        granted_at: row.get(1)?,
                        revoked_at: row.get(2)?,
                    })
                })?
                .collect()
        })
        .await?)
}

/// Whether `node_id` may send this node errands.
///
/// Nothing asks yet: an errand needs somewhere to arrive, and the
/// endpoint that receives one is phase 3. This is the check it will
/// make, written and tested beside the grant it reads — the two are one
/// decision, and splitting them across phases is how the second half
/// gets written against a half-remembered version of the first.
#[allow(dead_code)]
pub async fn is_authorised(database: &SqliteDatabase, node_id: &str) -> bool {
    let node_id = node_id.to_string();
    let granted: SqliteResult<Option<i64>> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT 1 FROM authority WHERE \"node_id\" = ?1 AND \"revoked_at\" IS NULL",
                    [node_id],
                    |row| row.get(0),
                )
                .optional()
        })
        .await;

    match granted {
        Ok(found) => found.is_some(),
        // A grant nobody can read is not a grant. Refusing is the only
        // safe direction: the cost is an errand that has to be sent
        // again, and the alternative is obeying whoever asked.
        Err(error) => {
            tracing::warn!(%error, "could not read the authority table; refusing");
            false
        }
    }
}

/// Take instructions from a node, and keep what it takes to ask.
///
/// Two forms of the same secret, and they are not redundant. The hash
/// is the record of what was granted. The clear copy is this node's
/// **own credential** for calling that authority — errands are
/// collected, not delivered, so this side is the one that has to prove
/// who it is. See migration `0018`.
pub async fn grant(database: &SqliteDatabase, node_id: &str, token: &str) -> NetworkResult<()> {
    let (node_id, hash, secret) = (node_id.to_string(), sha256_hex(token), token.to_string());
    Ok(database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO authority \
                   (\"node_id\", \"token_hash\", \"secret\", \"granted_at\") \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (\"node_id\") DO UPDATE SET \
                   \"token_hash\" = excluded.\"token_hash\", \
                   \"secret\" = excluded.\"secret\", \
                   \"granted_at\" = excluded.\"granted_at\", \
                   \"revoked_at\" = NULL",
                (node_id, hash, secret, now_ms()),
            )?;
            Ok(())
        })
        .await?)
}

/// What this node presents when it asks `node_id` for work.
///
/// `None` for an authority granted before errands existed — there is
/// nothing to derive it from, and re-joining is what fixes it. Also
/// `None` once revoked, which is the point: a revoked authority is one
/// this node stops asking.
///
/// Nothing reads it yet: the cron that collects errands is what does,
/// and it lands with the meaning of a `host` errand.
#[allow(dead_code)]
pub async fn credential_for(database: &SqliteDatabase, node_id: &str) -> Option<String> {
    let node_id = node_id.to_string();
    let found: NetworkResult<Option<Option<String>>> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"secret\" FROM authority \
                     WHERE \"node_id\" = ?1 AND \"revoked_at\" IS NULL",
                    [node_id],
                    |row| row.get(0),
                )
                .optional()
        })
        .await
        .map_err(Into::into);

    match found {
        Ok(secret) => secret.flatten(),
        Err(error) => {
            tracing::warn!(%error, "could not read this node's credential for an authority");
            None
        }
    }
}

/// Stop taking errands from a node, without forgetting that it once
/// could. Joining must not be a one-way door.
pub async fn revoke(database: &SqliteDatabase, node_id: &str) -> NetworkResult<()> {
    let node_id = node_id.to_string();
    Ok(database
        .write(move |connection| {
            connection.execute(
                "UPDATE authority SET \"revoked_at\" = ?2 WHERE \"node_id\" = ?1",
                (node_id, now_ms()),
            )?;
            Ok(())
        })
        .await?)
}

// ---------- who claimed a name -------------------------------------------
//
// Written in phase 0 and used here: an authority tells an edge to serve
// a name, and this is what refuses the second claim on it. The rule is
// the reason there is no consensus in this design, and it was worth
// settling before anything depended on it.

/// Why a claim was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Somebody else already serves this name here.
    Claimed { by: Option<String> },
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Claimed { by: Some(node) } => {
                write!(f, "already claimed by {node}")
            }
            Self::Claimed { by: None } => write!(f, "already claimed by this node"),
        }
    }
}

/// Claim a name for `authority`, or say who has it.
///
/// A second claim is refused rather than merged or overwritten. Two
/// authorities pointing one hostname at different backends is not a
/// conflict a machine can resolve, and picking one silently would make
/// the wrong answer look like the right one.
///
/// Re-claiming a name you already hold succeeds: the instruction is
/// convergent, and an errand sent twice must not fail the second time.
pub async fn claim(
    database: &SqliteDatabase,
    name: &str,
    authority: Option<&str>,
) -> NetworkResult<Result<(), Refused>> {
    let held = holder(database, name).await?;
    if let Some(existing) = held {
        if existing.as_deref() != authority {
            return Ok(Err(Refused::Claimed { by: existing }));
        }
        return Ok(Ok(()));
    }

    let (name, authority) = (name.to_string(), authority.map(str::to_string));
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO claim (\"name\", \"authority_id\", \"claimed_at\") \
                 VALUES (?1, ?2, ?3)",
                (name, authority, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(Ok(()))
}

/// `Some(None)` means this node claimed it itself; `None` means nobody
/// has.
async fn holder(database: &SqliteDatabase, name: &str) -> NetworkResult<Option<Option<String>>> {
    let name = name.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"authority_id\" FROM claim WHERE \"name\" = ?1",
                    [name],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
        })
        .await?)
}

/// Every name this node holds **on another node's behalf**.
///
/// The certificate loop needs it: a name that arrived on an edge errand
/// has no local port row, so reading the `port` table alone would leave
/// the one thing this node was asked to do — answer HTTPS for somebody
/// else's service — served by the local authority's self-signed
/// certificate for ever.
///
/// Names claimed by this node itself are left out. Those already come
/// from the `port` table, and counting them twice would only make the
/// loop ask for the same certificate under two names for one thing.
pub async fn claimed_for_others(database: &SqliteDatabase) -> NetworkResult<Vec<String>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"name\" FROM claim \
                     WHERE \"authority_id\" IS NOT NULL ORDER BY \"name\"",
                )?
                .query_map([], |row| row.get(0))?
                .collect()
        })
        .await?)
}

pub async fn release(database: &SqliteDatabase, name: &str) -> NetworkResult<()> {
    let name = name.to_string();
    Ok(database
        .write(move |connection| {
            connection.execute("DELETE FROM claim WHERE \"name\" = ?1", [name])?;
            Ok(())
        })
        .await?)
}

/// Let go of every name held for an authority this node no longer
/// serves, and forget its route.
///
/// Revoking `edge` withdraws the consent, and until this existed it left
/// behind everything that consent had produced: the claim, the proxy
/// route, and — the one that costs — a certificate order repeating twice
/// a day for a name this node will never answer for, against an
/// authority that locks the account after five failed authorizations.
///
/// The withdrawing errand is the tidy path and it is not enough on its
/// own. It arrives only if the other node is still there, still knows,
/// and still reaches this one; a node revoking a grant is quite often
/// doing it *because* one of those stopped being true. So this is
/// convergent and asks only about now: a claim whose authority does not
/// grant `edge` today is a claim to release, however it was made.
///
/// Returns what it let go of, because a node quietly dropping a name it
/// was answering for is the kind of thing somebody should be able to
/// read in the log.
pub async fn release_ungranted(database: &SqliteDatabase) -> NetworkResult<Vec<String>> {
    let held: Vec<(String, String)> = database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"name\", \"authority_id\" FROM claim \
                     WHERE \"authority_id\" IS NOT NULL ORDER BY \"name\"",
                )?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .await?;

    let mut released = Vec::new();
    for (name, authority) in held {
        // `granted_to` reads the grant through what this node provides
        // now, so a switch turned off releases what was granted of it
        // without anybody revoking anything.
        if capability::granted_to(database, &authority)
            .await
            .contains(&capability::Capability::Edge)
        {
            continue;
        }
        crate::edge::routes::forget_for_other(database, &name).await?;
        release(database, &name).await?;
        tracing::info!(
            %name,
            %authority,
            "let go of a name: this node no longer serves that node"
        );
        released.push(name);
    }
    Ok(released)
}

fn sha256_hex(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    async fn database() -> SqliteDatabase {
        crate::db::open_in_memory().await.expect("open")
    }

    /// An administrator, because an enrolment is minted by somebody and
    /// the column that says who is a foreign key.
    pub(crate) async fn admin(database: &SqliteDatabase) -> String {
        let token = crate::accounts::issue_setup_token(database)
            .await
            .expect("token");
        crate::accounts::create_admin(database, &token, "admin", "a long passphrase here")
            .await
            .expect("admin")
            .id
    }

    fn node(id: &str, kind: Kind, endpoint: Option<&str>) -> Node {
        Node {
            id: id.into(),
            name: id.into(),
            kind,
            endpoint: endpoint.map(str::to_string),
            public_key: None,
            overlay_ip: None,
            is_self: false,
            last_seen_at: None,
            allows: Vec::new(),
            ca_pem: None,
        }
    }

    /// Being an edge is about being reachable, not about a setting. A
    /// node called public with nowhere to dial is one that has not
    /// finished joining, and sending it a hostname would publish a name
    /// that resolves to nothing.
    #[test]
    fn an_edge_needs_somewhere_to_be_dialled() {
        assert!(node("a", Kind::Public, Some("a.example.com:443")).may_be_edge());
        assert!(!node("b", Kind::Public, None).may_be_edge());
        assert!(!node("c", Kind::Private, Some("10.0.0.2:443")).may_be_edge());
    }

    /// Unknown reads as private: a node we cannot place must never be
    /// offered as somewhere to send the internet.
    #[test]
    fn a_kind_nobody_recognises_is_private() {
        assert_eq!(Kind::parse("public"), Kind::Public);
        assert_eq!(Kind::parse("private"), Kind::Private);
        assert_eq!(Kind::parse("edge"), Kind::Private);
        assert_eq!(Kind::parse(""), Kind::Private);
    }

    #[tokio::test]
    async fn a_node_is_saved_and_found() {
        let database = database().await;
        save(
            &database,
            &node("pub-1", Kind::Public, Some("a.example.com:443")),
        )
        .await
        .expect("save");

        let found = find(&database, "pub-1")
            .await
            .expect("query")
            .expect("there");
        assert_eq!(found.kind, Kind::Public);
        assert!(found.may_be_edge());
        assert_eq!(all(&database).await.expect("query").len(), 1);
    }

    /// The row is what makes the model real rather than described, and
    /// re-running an install must not mint a second identity: the id is
    /// what every other node calls this one.
    #[tokio::test]
    async fn this_node_gets_one_row_and_keeps_its_id() {
        let database = database().await;
        let config = Config::default();

        let first = ensure_self(&database, &config).await.expect("seeded");
        let again = ensure_self(&database, &config).await.expect("converged");

        assert_eq!(first.id, again.id);
        assert!(first.is_self);
        assert_eq!(all(&database).await.expect("query").len(), 1);
        assert_eq!(me(&database).await.expect("query").as_ref(), Some(&again));
    }

    /// The node somebody is looking at is the one every other row is
    /// being compared against. Alphabetical alone put it second on a
    /// list of two, which is where this was noticed.
    #[tokio::test]
    async fn the_node_you_are_on_comes_first() {
        let database = database().await;
        crate::node::settings::set_domain(&database, Some("wabot-deploy-testing.example"))
            .await
            .expect("set");
        ensure_self(&database, &Config::default())
            .await
            .expect("seeded");
        // Sorts before it by name, which is the case that caught this.
        save(&database, &node("nd-joined", Kind::Private, None))
            .await
            .expect("save");

        let listed = all(&database).await.expect("query");
        assert!(listed[0].is_self, "{listed:?}");
    }

    /// Reachability, not a setting: a node with a name the world can
    /// dial is public, and clearing that name makes it private again
    /// whatever it used to be.
    #[tokio::test]
    async fn what_this_node_is_follows_the_name_it_answers_to() {
        let database = database().await;
        let config = Config::default();

        let nameless = ensure_self(&database, &config).await.expect("seeded");
        assert_eq!(nameless.kind, Kind::Private);
        assert!(!nameless.may_be_edge());

        crate::node::settings::set_domain(&database, Some("node.example"))
            .await
            .expect("set");
        let named = ensure_self(&database, &config).await.expect("converged");
        assert!(named.may_be_edge());
        assert_eq!(
            named.endpoint.as_deref(),
            Some(&*format!("node.example:{}", config.edge.https_port))
        );

        crate::node::settings::set_domain(&database, None)
            .await
            .expect("clear");
        assert_eq!(
            ensure_self(&database, &config)
                .await
                .expect("converged")
                .kind,
            Kind::Private
        );
    }

    /// Whatever a node learned about the overlay survives the next
    /// start. Seeding used to be the whole row, which would hand a
    /// joined node's address back to the allocator on every restart.
    #[tokio::test]
    async fn seeding_does_not_forget_the_overlay() {
        let database = database().await;
        let config = Config::default();

        let seeded = ensure_self(&database, &config).await.expect("seeded");
        let mut joined = seeded.clone();
        joined.overlay_ip = Some("10.42.0.7".into());
        save(&database, &joined).await.expect("save");
        keys::ensure(&database).await.expect("keys");

        let restarted = ensure_self(&database, &config).await.expect("converged");
        assert_eq!(restarted.overlay_ip.as_deref(), Some("10.42.0.7"));
        assert_eq!(
            restarted.public_key,
            keys::public_key(&database).await,
            "and it picks up the key once there is one"
        );
    }

    /// Forgetting is one direction, and it must not be a way to remove
    /// the row that says which node this is.
    #[tokio::test]
    async fn a_node_can_be_forgotten_and_this_one_cannot() {
        let database = database().await;
        let config = Config::default();
        let me = ensure_self(&database, &config).await.expect("seeded");
        save(&database, &node("pub-1", Kind::Public, Some("a:443")))
            .await
            .expect("save");

        forget(&database, "pub-1").await.expect("forget");
        assert_eq!(find(&database, "pub-1").await.expect("query"), None);

        forget(&database, &me.id).await.expect("refused quietly");
        assert!(
            find(&database, &me.id).await.expect("query").is_some(),
            "this node deleted itself"
        );
    }

    /// "This used to be allowed" is what somebody comes to the page to
    /// check, so a revoked grant stays listed rather than vanishing.
    #[tokio::test]
    async fn the_list_of_authorities_keeps_the_revoked_ones() {
        let database = database().await;
        grant(&database, "pub-1", "a-secret").await.expect("grant");
        grant(&database, "pub-2", "another").await.expect("grant");
        revoke(&database, "pub-2").await.expect("revoke");

        let authorities = authorities(&database).await.expect("query");
        assert_eq!(authorities.len(), 2);
        assert_eq!(authorities.iter().filter(|a| a.live()).count(), 1);
    }

    /// Nothing may configure this node until it says so. That is the
    /// whole model: an errand from a stranger is not a conflict to
    /// resolve, it is a request with no standing.
    #[tokio::test]
    async fn a_stranger_may_configure_nothing() {
        let database = database().await;
        assert!(!is_authorised(&database, "pub-1").await);

        grant(&database, "pub-1", "a-secret").await.expect("grant");
        assert!(is_authorised(&database, "pub-1").await);
        assert!(
            !is_authorised(&database, "pub-2").await,
            "and only the one that was granted"
        );
    }

    /// Revoking leaves the row, so "this used to be allowed" stays
    /// readable — and joining is not a one-way door.
    #[tokio::test]
    async fn a_grant_can_be_taken_back_and_given_again() {
        let database = database().await;
        grant(&database, "pub-1", "a-secret").await.expect("grant");

        revoke(&database, "pub-1").await.expect("revoke");
        assert!(!is_authorised(&database, "pub-1").await);

        grant(&database, "pub-1", "another").await.expect("regrant");
        assert!(is_authorised(&database, "pub-1").await, "and again");
    }

    /// The rule that keeps this out of consensus: one authority per
    /// name, and the second is refused rather than merged. Choosing
    /// silently would make the wrong backend look like the right one.
    #[tokio::test]
    async fn a_name_belongs_to_one_authority() {
        let database = database().await;

        assert!(claim(&database, "api.example.com", Some("pub-1"))
            .await
            .expect("claim")
            .is_ok());

        let refused = claim(&database, "api.example.com", Some("pub-2"))
            .await
            .expect("claim")
            .expect_err("refused");
        assert_eq!(
            refused,
            Refused::Claimed {
                by: Some("pub-1".into())
            }
        );
        assert!(
            refused.to_string().contains("pub-1"),
            "and it names who has it: {refused}"
        );
    }

    /// An errand sent twice must not fail the second time — the whole
    /// install story here is convergent, and a retry is not a conflict.
    #[tokio::test]
    async fn claiming_a_name_you_already_hold_is_fine() {
        let database = database().await;
        for _ in 0..3 {
            assert!(claim(&database, "api.example.com", Some("pub-1"))
                .await
                .expect("claim")
                .is_ok());
        }

        release(&database, "api.example.com")
            .await
            .expect("release");
        assert!(
            claim(&database, "api.example.com", Some("pub-2"))
                .await
                .expect("claim")
                .is_ok(),
            "and releasing lets somebody else have it"
        );
    }

    /// The withdrawing errand is the tidy path and it cannot be the only
    /// one: it arrives only if the other node is still there, still
    /// knows and still reaches this one — and a node revoking a grant
    /// is often doing it because one of those stopped being true.
    ///
    /// So this asks about now. A name held for a node this one no
    /// longer serves is let go of at the next boot, and the certificate
    /// loop stops ordering for it.
    #[tokio::test]
    async fn a_name_held_for_a_node_this_one_no_longer_serves_is_let_go_of() {
        let database = database().await;

        capability::grant(&database, "pub-1", &[capability::Capability::Edge])
            .await
            .expect("grant");
        capability::grant(&database, "pub-2", &[capability::Capability::Edge])
            .await
            .expect("grant");
        for (name, authority) in [("gone.example", "pub-1"), ("kept.example", "pub-2")] {
            claim(&database, name, Some(authority))
                .await
                .expect("claim")
                .expect("free");
            crate::edge::routes::upsert(
                &database,
                name,
                &crate::edge::routes::Upstream::Proxy(vec!["10.42.0.1:30001".parse().expect("ok")]),
                None,
            )
            .await
            .expect("route");
        }

        capability::grant(&database, "pub-1", &[])
            .await
            .expect("revoke");
        let released = release_ungranted(&database).await.expect("released");

        assert_eq!(released, vec!["gone.example".to_string()]);
        assert_eq!(
            claimed_for_others(&database).await.expect("claims"),
            vec!["kept.example".to_string()],
            "a name held for a node this one still serves was taken too"
        );
        let routes: Vec<String> = crate::edge::routes::load_all(&database)
            .await
            .expect("routes")
            .into_iter()
            .map(|(host, _)| host)
            .collect();
        assert!(
            !routes.iter().any(|host| host == "gone.example"),
            "the listener would still proxy for it: {routes:?}"
        );
        assert!(
            routes.iter().any(|host| host == "kept.example"),
            "and the one still granted was dropped too: {routes:?}"
        );
    }
}
