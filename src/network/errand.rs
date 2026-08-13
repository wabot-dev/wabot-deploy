//! What an authority has asked another node to do.
//!
//! ## The queue is on the node giving the order
//!
//! An errand is written here and *collected* by the node it is for.
//! That is the whole delivery mechanism, and it is the direction that
//! works: a private node dials out over the certificate it already
//! trusts — the one it enrolled through — so nothing ever has to reach
//! it. Which is the reason private nodes exist.
//!
//! It also means the overlay is not in this path. The overlay is a data
//! plane: it exists so an edge can reach a *container* on another node.
//! Orders are control plane.
//!
//! ## An errand is an instruction, not a job
//!
//! The node that collects one writes its own local job for it. So there
//! is no distributed queue and no job routing — `deploy` still talks to
//! this node's containerd, whichever node that turns out to be. See
//! migration `0017`.
//!
//! ## Half of it is wired
//!
//! The authority's two endpoints hand errands over and take the answer
//! back. What is missing is the other end — the collecting node's cron,
//! and what a `host` errand *means* there: its own service row, its own
//! pull, its own local deploy job. That is what the `allow`s below name.

use serde::{Deserialize, Serialize};
use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::SqliteDatabase;

use super::NetworkResult;
use crate::platform::now_ms;

/// What kind of instruction this is.
///
/// One today. The enum exists rather than a bare string because the
/// node reading it is running a different version than the node that
/// wrote it as often as not — an unrecognised kind has to be a value
/// the code can hold and refuse, not a parse failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Run this service here.
    Host,
    /// Serve this name from here, proxying to the replicas named.
    Edge,
    /// Keep a read-only copy of a database here.
    ///
    /// A kind of its own rather than a field on [`Kind::Host`], and the
    /// reason is what an older node would do with it: serde ignores a
    /// field it does not know, so a `host` errand carrying database
    /// arguments would be run as a plain container — a Postgres with no
    /// volume, initialising into a layer that is thrown away at the next
    /// deployment. It would look like it worked. An unknown *kind* is
    /// refused with a reason instead, which is what that machinery is
    /// for.
    Database,
    /// Anything this version does not know about.
    #[serde(other)]
    Unknown,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Edge => "edge",
            Self::Database => "database",
            Self::Unknown => "unknown",
        }
    }

    /// A kind from a newer node reads as [`Kind::Unknown`] rather than
    /// as a failure. The node refuses it with a reason the operator can
    /// act on — "that node is older than this instruction" — instead of
    /// dropping an errand nobody can explain.
    pub fn parse(text: &str) -> Self {
        match text {
            "host" => Self::Host,
            "edge" => Self::Edge,
            "database" => Self::Database,
            _ => Self::Unknown,
        }
    }
}

/// One instruction, as both ends see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Errand {
    pub id: String,
    pub kind: Kind,
    /// The kind's own arguments, still as JSON. This module knows the
    /// *shape* of an errand — both ends have to agree on that — and
    /// deliberately not what to do about one. Carrying and obeying are
    /// different jobs.
    pub payload: serde_json::Value,
}

/// The arguments of a [`Kind::Host`] errand: run this service here.
///
/// Everything the far node needs and nothing it can look up, because it
/// cannot look anything up — it has never heard of this project, this
/// service or this registry until the errand arrives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    /// What to call the project there. Its own project, with its own
    /// bridge and its own rows — nothing is shared, which is the whole
    /// model.
    pub project: String,
    pub service: String,
    /// Pinned by digest by whoever queued it. A tag would mean the two
    /// nodes could resolve the same errand to different bytes.
    pub image: String,
    /// The registry host in `image`. The far node stores the credential
    /// below against it — see `platform::registry_credentials`.
    pub registry: String,
    /// What to present there, when there is anything to present.
    ///
    /// **Absent for a registry that is not this node's own.** These
    /// used to be required, and what went into them was a token for
    /// *this* node's registry whatever host the image named — so
    /// placing `docker.io/library/postgres` on another node would have
    /// handed a wabot push token to Docker Hub. Nobody had placed a
    /// public image elsewhere, so nobody had; a database pulls from
    /// Docker Hub by default, so somebody would have.
    ///
    /// Omitted rather than sent empty. A node old enough to require
    /// them refuses the errand and says so, which is a reason somebody
    /// can act on — where an empty password is a pull that fails at the
    /// registry with an authentication error about a credential nobody
    /// meant to send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// What the container should listen on. `None` leaves it to the
    /// image's own declaration.
    #[serde(default)]
    pub port: Option<u16>,
    /// Which copies of the service this node is to run, by the
    /// **service's** numbering rather than its own.
    ///
    /// A node given slots 2 and 3 runs slots 2 and 3, so the container
    /// ids match on both sides and a report about "slot 3" names one
    /// thing across the network. Defaulted for a payload written before
    /// this field existed, which meant exactly one copy.
    #[serde(default = "one_slot")]
    pub slots: Vec<u32>,
}

fn one_slot() -> Vec<u32> {
    vec![1]
}

/// The arguments of a [`Kind::Edge`] errand: answer for this name here.
///
/// The upstreams come *with* the instruction rather than being looked
/// up: the node that placed the replicas knows where they are and
/// whether they answered, and an edge that discovered them would be a
/// second thing with an opinion about where a service is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub hostname: String,
    /// `host:port` per replica, and **one entry per replica** — a node
    /// running two copies appears twice, which is what makes a plain
    /// round-robin send it twice the requests.
    ///
    /// Each is an overlay address and a port bound to it, never a
    /// container's own: a bridge address is not unique across nodes.
    pub upstreams: Vec<String>,
}

/// The arguments of a [`Kind::Database`] errand: hold a copy here.
///
/// Everything the far node needs and nothing it can look up. It has
/// never heard of this database, and — unlike a `host` errand — it
/// cannot work out the one thing that matters most from its own rows:
/// **where the primary answers**. That address is on the sending node's
/// overlay, and the port came out of that node's own port space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Standby {
    pub project: String,
    pub service: String,
    /// Pinned by whoever queued it, like a `host` errand's.
    pub image: String,
    pub registry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,

    /// The ceiling, in bytes. Sent rather than chosen there: a copy that
    /// ran with different settings from its primary would be a different
    /// database wearing the same name.
    pub memory_limit: u64,
    pub engine: String,
    pub version: String,

    /// The credentials, in full. A standby is seeded by connecting to
    /// the primary as the replication role, and it serves reads to
    /// clients that authenticate as the ordinary one — so it needs both,
    /// and both have to be the *same* as the primary's or the copy is
    /// not the same database.
    pub database_name: String,
    pub admin_user: String,
    pub admin_password: String,
    pub replication_user: String,
    pub replication_password: String,

    /// Where the primary answers, as `host:port` — the sending node's
    /// overlay address and a port bound to it. Never a container's own:
    /// a bridge address names a different container on every machine,
    /// which is the rule phase 7 already learned.
    pub primary: String,
    /// Which copies to hold, in the **service's** numbering. Never the
    /// primary's slot: this errand only ever asks for standbys.
    pub slots: Vec<u32>,
    pub primary_slot: u32,
    /// The **owner's** domain, so the copy answers to the database's own
    /// qualified name rather than to one built from the holding node's.
    ///
    /// Without it the same database had a different long name on every
    /// machine that held a copy, each with its own certificate — so no
    /// single connection string reached it, which is the whole point of
    /// a qualified name. Absent when the owner has no domain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_domain: Option<String>,
}

/// An errand and what became of it, for the page that lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub id: String,
    pub node_id: String,
    pub kind: Kind,
    /// What it was about, as it was queued.
    ///
    /// Carried on the record because the page that lists errands has to
    /// say *which* one: "an edge errand to that node" is not an answer
    /// when the same node serves two of this service's names, and the
    /// row somebody is looking at is about one of them.
    pub payload: serde_json::Value,
    pub created_at: i64,
    pub taken_at: Option<i64>,
    pub done_at: Option<i64>,
    pub error: Option<String>,
}

/// Read by the page that will list errands, and by the tests that
/// pin what settling means.
#[allow(dead_code)]
impl Record {
    pub fn done(&self) -> bool {
        self.done_at.is_some()
    }

    /// Whether it finished badly. A failure is an *answer* — the state
    /// worth worrying about is one that never came back at all.
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }
}

/// Ask a node to do something.
///
/// Nothing queues one yet: the console form that does is what removes
/// this, and it arrives with the meaning of a `host` errand.
#[allow(dead_code)]
pub async fn queue(
    database: &SqliteDatabase,
    node_id: &str,
    kind: Kind,
    payload: &serde_json::Value,
) -> NetworkResult<Errand> {
    queue_about(database, node_id, kind, None, payload).await
}

async fn queue_about(
    database: &SqliteDatabase,
    node_id: &str,
    kind: Kind,
    subject: Option<&str>,
    payload: &serde_json::Value,
) -> NetworkResult<Errand> {
    let errand = Errand {
        id: format!("er-{}", wabot::prelude::password::generate(12)),
        kind,
        payload: payload.clone(),
    };

    let (id, node, kind) = (
        errand.id.clone(),
        node_id.to_string(),
        errand.kind.as_str().to_string(),
    );
    let (body, about) = (payload.to_string(), subject.map(str::to_string));
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO errand \
                   (\"id\", \"node_id\", \"kind\", \"payload\", \"subject\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (id, node, kind, body, about, now_ms()),
            )?;
            Ok(())
        })
        .await?;

    Ok(errand)
}

/// Ask a node to do something, unless it was already asked the same
/// thing.
///
/// **Because a database's errands are recomputed rather than emitted.**
/// Every other errand is queued by somebody pressing a button, once. A
/// database's cannot be: the port its primary answers on comes out of
/// the other node's port space and arrives home on a report, so the
/// instruction has to be rebuilt whenever the facts settle — and a pass
/// that rebuilt it every fifteen seconds would queue an errand every
/// fifteen seconds.
///
/// `subject` is what the instruction is about, so "the same thing" is
/// answerable: one node can hold standbys of two databases, and the most
/// recent errand of a kind is not enough to tell them apart.
///
/// An unchanged payload is skipped **whatever became of the last one**,
/// including a failure. A failure is an answer and this node's rule is
/// that retrying is something somebody asks for; requeueing it here
/// would be an automatic retry loop wearing a different name.
pub async fn queue_if_changed(
    database: &SqliteDatabase,
    node_id: &str,
    kind: Kind,
    subject: &str,
    payload: &serde_json::Value,
) -> NetworkResult<Option<Errand>> {
    let (node, about) = (node_id.to_string(), subject.to_string());
    let last: Option<String> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"payload\" FROM errand \
                     WHERE \"node_id\" = ?1 AND \"subject\" = ?2 \
                     ORDER BY \"created_at\" DESC LIMIT 1",
                    (node, about),
                    |row| row.get(0),
                )
                .optional()
        })
        .await?;

    if let Some(last) = last {
        if serde_json::from_str::<serde_json::Value>(&last)
            .ok()
            .as_ref()
            == Some(payload)
        {
            return Ok(None);
        }
    }
    queue_about(database, node_id, kind, Some(subject), payload)
        .await
        .map(Some)
}

/// What is waiting for a node, oldest first.
///
/// Handing the same errand over twice is normal and deliberate: a node
/// that collected one and then died has to be given it again, and there
/// is no way to tell that case from a slow one. So collecting is not
/// what settles an errand — saying it is finished is.
pub async fn waiting(database: &SqliteDatabase, node_id: &str) -> NetworkResult<Vec<Errand>> {
    let node = node_id.to_string();
    let rows: Vec<(String, String, String)> = database
        .read(move |connection| {
            let mut statement = connection.prepare(
                "SELECT \"id\", \"kind\", \"payload\" FROM errand \
                 WHERE \"node_id\" = ?1 AND \"done_at\" IS NULL \
                 ORDER BY \"created_at\"",
            )?;
            let rows: wabot::sqlite::rusqlite::Result<Vec<(String, String, String)>> = statement
                .query_map([node], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect();
            rows
        })
        .await?;

    let ids: Vec<String> = rows.iter().map(|(id, _, _)| id.clone()).collect();
    if !ids.is_empty() {
        mark_taken(database, &ids).await?;
    }

    Ok(rows
        .into_iter()
        .map(|(id, kind, payload)| Errand {
            id,
            kind: Kind::parse(&kind),
            // A payload that will not parse is handed over as null
            // rather than dropped: the node refuses it and says so,
            // which is a reason somebody can act on. Silently skipping
            // it would be an errand that never arrives and never fails.
            payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
        })
        .collect())
}

async fn mark_taken(database: &SqliteDatabase, ids: &[String]) -> NetworkResult<()> {
    let ids = ids.to_vec();
    database
        .write(move |connection| {
            for id in ids {
                connection.execute(
                    "UPDATE errand SET \"taken_at\" = ?2 WHERE \"id\" = ?1",
                    (id, now_ms()),
                )?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

/// Record how it went.
///
/// Both outcomes settle it. An errand that failed is finished — the
/// authority knows, with the reason — and retrying is a new errand
/// somebody asked for, not this one quietly coming back.
///
/// Scoped to the node it belongs to, so one node cannot settle
/// another's errand by guessing an id.
pub async fn settle(
    database: &SqliteDatabase,
    node_id: &str,
    id: &str,
    error: Option<&str>,
) -> NetworkResult<bool> {
    let (id, node, error) = (
        id.to_string(),
        node_id.to_string(),
        error.map(str::to_string),
    );
    let settled = database
        .write(move |connection| {
            connection.execute(
                "UPDATE errand SET \"done_at\" = ?3, \"error\" = ?4 \
                 WHERE \"id\" = ?1 AND \"node_id\" = ?2 AND \"done_at\" IS NULL",
                (id, node, now_ms(), error),
            )
        })
        .await?;
    Ok(settled > 0)
}

/// Every errand, newest first. For the page that will list them.
#[allow(dead_code)]
pub async fn all(database: &SqliteDatabase) -> NetworkResult<Vec<Record>> {
    Ok(database
        .read(|connection| {
            let mut statement = connection.prepare(
                "SELECT \"id\", \"node_id\", \"kind\", \"payload\", \"created_at\", \
                 \"taken_at\", \"done_at\", \"error\" FROM errand \
                 ORDER BY \"created_at\" DESC",
            )?;
            let records: wabot::sqlite::rusqlite::Result<Vec<Record>> =
                statement.query_map([], decode)?.collect();
            records
        })
        .await?)
}

#[allow(dead_code)]
pub async fn find(database: &SqliteDatabase, id: &str) -> NetworkResult<Option<Record>> {
    let id = id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"id\", \"node_id\", \"kind\", \"payload\", \"created_at\", \
                     \"taken_at\", \"done_at\", \"error\" FROM errand WHERE \"id\" = ?1",
                    [id],
                    decode,
                )
                .optional()
        })
        .await?)
}

fn decode(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Record> {
    Ok(Record {
        id: row.get(0)?,
        node_id: row.get(1)?,
        kind: Kind::parse(&row.get::<_, String>(2)?),
        // A payload this version cannot parse reads as null rather than
        // failing the row: the page still has an errand to show, and
        // "something was asked and here is how it went" is more use
        // than an empty table.
        payload: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
        created_at: row.get(4)?,
        taken_at: row.get(5)?,
        done_at: row.get(6)?,
        error: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> SqliteDatabase {
        crate::db::open_in_memory().await.expect("open")
    }

    fn payload() -> serde_json::Value {
        serde_json::json!({ "service": "web", "image": "hub/proj/app@sha256:abc" })
    }

    #[tokio::test]
    async fn an_errand_waits_for_the_node_it_is_for() {
        let database = database().await;
        queue(&database, "nd-one", Kind::Host, &payload())
            .await
            .expect("queued");

        let collected = waiting(&database, "nd-one").await.expect("waiting");
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].kind, Kind::Host);
        assert_eq!(collected[0].payload, payload());

        assert!(
            waiting(&database, "nd-two")
                .await
                .expect("waiting")
                .is_empty(),
            "another node collected an errand that was not for it"
        );
    }

    /// Collecting is not what settles an errand. A node that fetched one
    /// and then died has to be given it again, and nothing here can tell
    /// that case from a slow one.
    #[tokio::test]
    async fn collecting_twice_hands_it_over_twice() {
        let database = database().await;
        let queued = queue(&database, "nd-one", Kind::Host, &payload())
            .await
            .expect("queued");

        for _ in 0..3 {
            let collected = waiting(&database, "nd-one").await.expect("waiting");
            assert_eq!(collected.len(), 1, "it stopped being handed over");
            assert_eq!(collected[0].id, queued.id);
        }

        // And collecting is recorded, so "asked for and never finished"
        // is distinguishable from "never asked for".
        let record = find(&database, &queued.id)
            .await
            .expect("query")
            .expect("there");
        assert!(record.taken_at.is_some());
        assert!(!record.done());
    }

    /// Both outcomes settle it. A failure is an answer; the state worth
    /// worrying about is one that never came back at all.
    #[tokio::test]
    async fn either_outcome_finishes_it() {
        let database = database().await;
        for (node, error) in [("nd-one", None), ("nd-two", Some("no such image"))] {
            let queued = queue(&database, node, Kind::Host, &payload())
                .await
                .expect("queued");
            assert!(settle(&database, node, &queued.id, error)
                .await
                .expect("settle"));

            let record = find(&database, &queued.id)
                .await
                .expect("query")
                .expect("there");
            assert!(record.done());
            assert_eq!(record.failed(), error.is_some());
            assert!(waiting(&database, node).await.expect("waiting").is_empty());
        }
    }

    /// One node must not settle another's errand by guessing an id, and
    /// an errand already finished must not be reopened by a late
    /// acknowledgement from a node that was retrying.
    #[tokio::test]
    async fn an_errand_is_settled_once_and_only_by_its_own_node() {
        let database = database().await;
        let queued = queue(&database, "nd-one", Kind::Host, &payload())
            .await
            .expect("queued");

        assert!(
            !settle(&database, "nd-two", &queued.id, None)
                .await
                .expect("settle"),
            "another node settled it"
        );
        assert!(settle(&database, "nd-one", &queued.id, None)
            .await
            .expect("settle"));
        assert!(
            !settle(&database, "nd-one", &queued.id, Some("actually it failed"))
                .await
                .expect("settle"),
            "a finished errand was reopened"
        );

        let record = find(&database, &queued.id)
            .await
            .expect("query")
            .expect("there");
        assert!(!record.failed(), "the late failure overwrote the outcome");
    }

    /// The node reading an errand is running a different version than
    /// the node that wrote it as often as not. An instruction from the
    /// future has to be something the code can hold and refuse.
    #[test]
    fn an_instruction_this_version_does_not_know_is_a_value_not_a_crash() {
        assert_eq!(Kind::parse("host"), Kind::Host);
        assert_eq!(Kind::parse("edge"), Kind::Edge);
        // Something a later version might add. This example used to be
        // `edge`, which is the point: today's unknown is tomorrow's
        // kind, and what has to keep working is the *shape* of not
        // knowing.
        assert_eq!(Kind::parse("backup"), Kind::Unknown);
        assert_eq!(Kind::parse(""), Kind::Unknown);

        // And over the wire, which is where it actually arrives.
        let from_the_future: Errand =
            serde_json::from_str(r#"{"id":"er-1","kind":"backup","payload":{}}"#).expect("decodes");
        assert_eq!(from_the_future.kind, Kind::Unknown);
    }

    #[tokio::test]
    async fn the_list_says_what_became_of_each_one() {
        let database = database().await;
        let first = queue(&database, "nd-one", Kind::Host, &payload())
            .await
            .expect("queued");
        queue(&database, "nd-one", Kind::Host, &payload())
            .await
            .expect("queued");
        settle(&database, "nd-one", &first.id, Some("refused"))
            .await
            .expect("settle");

        let all = all(&database).await.expect("list");
        assert_eq!(all.len(), 2);
        assert_eq!(all.iter().filter(|r| r.done()).count(), 1);
        assert_eq!(all.iter().filter(|r| r.failed()).count(), 1);
    }
}
