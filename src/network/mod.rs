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
//! ## Nothing calls this yet
//!
//! This is the model on its own — the shape everything else in the
//! network work hangs off, written and tested before anything is built
//! on top of it. `install` seeding the self row and the console reading
//! this table instead of a synthetic list of one are the next step, and
//! the `allow` below goes when they land.
//!
//! Writing it first is deliberate: the claim rule and the direction of
//! authority are the two decisions that would be expensive to change
//! later, and they are cheap to argue about now.
//!
//! ## Public and private is about reachability, not policy
//!
//! A public node has an address the internet can dial, which is what
//! lets it terminate TLS for a name whose container runs elsewhere. A
//! private node does not. That is the entire difference, and it is why
//! [`Kind`] is derived from the endpoint rather than trusted from a
//! setting somebody could set wrongly.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::{SqliteDatabase, SqliteResult};

use crate::platform::now_ms;

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

pub async fn all(database: &SqliteDatabase) -> SqliteResult<Vec<Node>> {
    database
        .read(|connection| {
            let mut statement =
                connection.prepare(&format!("SELECT {COLUMNS} FROM node ORDER BY \"name\""))?;
            let nodes: wabot::sqlite::rusqlite::Result<Vec<Node>> =
                statement.query_map([], read)?.collect();
            nodes
        })
        .await
}

pub async fn find(database: &SqliteDatabase, id: &str) -> SqliteResult<Option<Node>> {
    let id = id.to_string();
    database
        .read(move |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM node WHERE \"id\" = ?1"),
                    [id],
                    read,
                )
                .optional()
        })
        .await
}

/// The node this process is.
pub async fn me(database: &SqliteDatabase) -> SqliteResult<Option<Node>> {
    database
        .read(|connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM node WHERE \"is_self\" = 1"),
                    [],
                    read,
                )
                .optional()
        })
        .await
}

/// Write a node, or update what is known about one.
///
/// Convergent, like every install step: it asks about the node, not
/// about whether it has been seen before.
pub async fn save(database: &SqliteDatabase, node: &Node) -> SqliteResult<()> {
    let node = node.clone();
    database
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
        .await
}

// ---------- who may configure this node ---------------------------------

/// Whether `node_id` may send this node errands.
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

pub async fn grant(database: &SqliteDatabase, node_id: &str, token: &str) -> SqliteResult<()> {
    let (node_id, hash) = (node_id.to_string(), sha256_hex(token));
    database
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
        .await
}

/// Stop taking errands from a node, without forgetting that it once
/// could. Joining must not be a one-way door.
pub async fn revoke(database: &SqliteDatabase, node_id: &str) -> SqliteResult<()> {
    let node_id = node_id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE authority SET \"revoked_at\" = ?2 WHERE \"node_id\" = ?1",
                (node_id, now_ms()),
            )?;
            Ok(())
        })
        .await
}

// ---------- who claimed a name -------------------------------------------

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
) -> SqliteResult<Result<(), Refused>> {
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
async fn holder(database: &SqliteDatabase, name: &str) -> SqliteResult<Option<Option<String>>> {
    let name = name.to_string();
    database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"authority_id\" FROM claim WHERE \"name\" = ?1",
                    [name],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
        })
        .await
}

pub async fn release(database: &SqliteDatabase, name: &str) -> SqliteResult<()> {
    let name = name.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM claim WHERE \"name\" = ?1", [name])?;
            Ok(())
        })
        .await
}

fn sha256_hex(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> SqliteDatabase {
        crate::db::open_in_memory().await.expect("open")
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
