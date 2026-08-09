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
//! ## Public and private is about reachability, not policy
//!
//! A public node has an address the internet can dial, which is what
//! lets it terminate TLS for a name whose container runs elsewhere. A
//! private node does not. That is the entire difference, and it is why
//! [`Kind`] is derived from the endpoint rather than trusted from a
//! setting somebody could set wrongly.

pub mod api;
pub mod call;
pub mod enrolment;
pub mod join;
pub mod keys;
pub mod overlay;
pub mod token;

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
    /// Where the world dials it. `None` for a private node — that
    /// absence *is* what makes it private.
    pub endpoint: Option<String>,
    /// Filled when it joins the overlay.
    pub public_key: Option<String>,
    pub overlay_ip: Option<String>,
    pub is_self: bool,
    pub last_seen_at: Option<i64>,
}

impl Node {
    /// Whether this node can be asked to serve traffic for a name.
    ///
    /// Both halves are required and neither is a setting: an endpoint
    /// the world can dial, and a kind that says so. A public node
    /// without an endpoint is one that has not finished being set up,
    /// and sending a hostname to it would be a name that resolves to
    /// nothing.
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
    })
}

const COLUMNS: &str = "\"id\", \"name\", \"kind\", \"endpoint\", \"public_key\", \
                       \"overlay_ip\", \"is_self\", \"last_seen_at\"";

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
pub async fn ensure_self(database: &SqliteDatabase, config: &Config) -> NetworkResult<Node> {
    let existing = me(database).await?;
    let domain = crate::node::settings::domain(database, config).await;

    // Derived, not stored: a node answers to a name or it does not, and
    // an operator who cleared the domain has made this node private
    // whatever it used to be.
    let endpoint = domain
        .as_ref()
        .map(|domain| format!("{domain}:{}", config.edge.https_port));
    let kind = match endpoint {
        Some(_) => Kind::Public,
        None => Kind::Private,
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
            let mut statement =
                connection.prepare(&format!("SELECT {COLUMNS} FROM node ORDER BY \"name\""))?;
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
                    \"is_self\", \"joined_at\", \"last_seen_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
                 ON CONFLICT (\"id\") DO UPDATE SET \
                   \"name\" = excluded.\"name\", \
                   \"kind\" = excluded.\"kind\", \
                   \"endpoint\" = excluded.\"endpoint\", \
                   \"public_key\" = excluded.\"public_key\", \
                   \"overlay_ip\" = excluded.\"overlay_ip\", \
                   \"last_seen_at\" = excluded.\"last_seen_at\"",
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

pub async fn grant(database: &SqliteDatabase, node_id: &str, token: &str) -> NetworkResult<()> {
    let (node_id, hash) = (node_id.to_string(), sha256_hex(token));
    Ok(database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO authority (\"node_id\", \"token_hash\", \"granted_at\") \
                 VALUES (?1, ?2, ?3) \
                 ON CONFLICT (\"node_id\") DO UPDATE SET \
                   \"token_hash\" = excluded.\"token_hash\", \
                   \"granted_at\" = excluded.\"granted_at\", \
                   \"revoked_at\" = NULL",
                (node_id, hash, now_ms()),
            )?;
            Ok(())
        })
        .await?)
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
// Phase 4: an authority tells an edge to route a name to a container
// somewhere else, and this is what refuses the second claim on it. The
// rule is the reason there is no consensus here, so it is written and
// tested now rather than argued about later with a working system in
// the way. Nothing calls it until there is an errand to carry it.

/// Why a claim was not accepted.
#[allow(dead_code)]
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
#[allow(dead_code)]
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

#[allow(dead_code)]
pub async fn release(database: &SqliteDatabase, name: &str) -> NetworkResult<()> {
    let name = name.to_string();
    Ok(database
        .write(move |connection| {
            connection.execute("DELETE FROM claim WHERE \"name\" = ?1", [name])?;
            Ok(())
        })
        .await?)
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
}
