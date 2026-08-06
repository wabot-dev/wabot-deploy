//! The install ledger: which steps have run, and how they went.
//!
//! `install` is a sequence of steps that must converge rather than
//! repeat. Each one records itself here before and after, so a second
//! run skips what is already done and a run that died halfway can say
//! where — which is the difference between "run it again" and "work
//! out what state the machine is in".

use serde::Serialize;
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::{SqliteDatabase, SqliteResult};

/// Every step `install` can take, in the order it takes them.
///
/// An enum rather than free strings: the ledger is read back by
/// `doctor` and by the next install, and a step renamed in one place
/// and not the other would silently re-run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Preflight,
    Layout,
    Config,
    Database,
    Runtime,
    Binary,
    Service,
    Certificate,
    Start,
}

impl Step {
    pub fn as_str(self) -> &'static str {
        match self {
            Step::Preflight => "preflight",
            Step::Layout => "layout",
            Step::Config => "config",
            Step::Database => "database",
            Step::Runtime => "runtime",
            Step::Binary => "binary",
            Step::Service => "service",
            Step::Certificate => "certificate",
            Step::Start => "start",
        }
    }

    /// What `install` runs today. The rest are declared because the
    /// ledger is also a plan, and `doctor` listing them as pending is
    /// more useful than pretending they do not exist.
    ///
    /// `Preflight` is absent on purpose: it changes nothing, so there
    /// is no state for it to record. `Certificate` is absent because
    /// it is attempted and allowed to fail — a node with no public
    /// certificate is still installed.
    pub const IMPLEMENTED: &'static [Step] = &[
        Step::Layout,
        Step::Config,
        Step::Database,
        Step::Runtime,
        Step::Binary,
        Step::Service,
        Step::Start,
    ];

    pub const ALL: &'static [Step] = &[
        Step::Preflight,
        Step::Layout,
        Step::Config,
        Step::Database,
        Step::Runtime,
        Step::Binary,
        Step::Service,
        Step::Certificate,
        Step::Start,
    ];
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Started and not yet finished. A row left in this state is a run
    /// that died — which is exactly what the next one needs to know.
    Running,
    Done,
    Failed,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Running => "running",
            Status::Done => "done",
            Status::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "running" => Some(Status::Running),
            "done" => Some(Status::Done),
            "failed" => Some(Status::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub step: String,
    pub status: Status,
    pub detail: Option<String>,
    pub updated_at: i64,
}

pub async fn record(
    database: &SqliteDatabase,
    step: Step,
    status: Status,
    detail: Option<String>,
) -> SqliteResult<()> {
    let (step, status) = (step.as_str().to_string(), status.as_str().to_string());
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO node_state (\"step\", \"status\", \"detail\", \"updated_at\") \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (\"step\") DO UPDATE SET \
                   \"status\" = excluded.\"status\", \
                   \"detail\" = excluded.\"detail\", \
                   \"updated_at\" = excluded.\"updated_at\"",
                (step, status, detail, now_ms()),
            )?;
            Ok(())
        })
        .await
}

pub async fn entry(database: &SqliteDatabase, step: Step) -> SqliteResult<Option<Entry>> {
    let key = step.as_str().to_string();
    database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"step\", \"status\", \"detail\", \"updated_at\" \
                     FROM node_state WHERE \"step\" = ?1",
                    [key],
                    |row| {
                        Ok(Entry {
                            step: row.get(0)?,
                            status: Status::parse(&row.get::<_, String>(1)?)
                                .unwrap_or(Status::Failed),
                            detail: row.get(2)?,
                            updated_at: row.get(3)?,
                        })
                    },
                )
                .optional()
        })
        .await
}

/// Has this step already finished successfully?
///
/// The question every step asks first. A `Running` row answers `false`
/// — a step that died mid-way has to be redone, and treating it as
/// finished is how a half-installed node looks healthy.
pub async fn is_done(database: &SqliteDatabase, step: Step) -> SqliteResult<bool> {
    Ok(entry(database, step)
        .await?
        .is_some_and(|entry| entry.status == Status::Done))
}

pub async fn all(database: &SqliteDatabase) -> SqliteResult<Vec<Entry>> {
    database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"step\", \"status\", \"detail\", \"updated_at\" \
                     FROM node_state ORDER BY \"updated_at\" ASC",
                )?
                .query_map([], |row| {
                    Ok(Entry {
                        step: row.get(0)?,
                        status: Status::parse(&row.get::<_, String>(1)?).unwrap_or(Status::Failed),
                        detail: row.get(2)?,
                        updated_at: row.get(3)?,
                    })
                })?
                .collect()
        })
        .await
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> SqliteDatabase {
        crate::db::open_in_memory().await.expect("open")
    }

    #[tokio::test]
    async fn a_step_is_not_done_until_it_says_so() {
        let database = database().await;
        assert!(!is_done(&database, Step::Layout).await.expect("is_done"));

        record(&database, Step::Layout, Status::Running, None)
            .await
            .expect("record");
        assert!(
            !is_done(&database, Step::Layout).await.expect("is_done"),
            "a step that started and did not finish must be redone"
        );

        record(&database, Step::Layout, Status::Done, None)
            .await
            .expect("record");
        assert!(is_done(&database, Step::Layout).await.expect("is_done"));
    }

    #[tokio::test]
    async fn a_failure_keeps_its_reason() {
        let database = database().await;
        record(
            &database,
            Step::Database,
            Status::Failed,
            Some("disk full".into()),
        )
        .await
        .expect("record");

        let entry = entry(&database, Step::Database)
            .await
            .expect("entry")
            .expect("present");
        assert_eq!(entry.status, Status::Failed);
        assert_eq!(entry.detail.as_deref(), Some("disk full"));
        assert!(!is_done(&database, Step::Database).await.expect("is_done"));
    }

    /// Re-recording replaces rather than accumulating: the ledger is
    /// the current state, not a history.
    #[tokio::test]
    async fn recording_twice_leaves_one_row() {
        let database = database().await;
        record(&database, Step::Config, Status::Running, None)
            .await
            .expect("record");
        record(&database, Step::Config, Status::Done, None)
            .await
            .expect("record");

        let entries = all(&database).await.expect("all");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, Status::Done);
    }

    /// Every declared step must have a distinct name, or two of them
    /// share a ledger row and each reports the other's outcome.
    #[test]
    fn step_names_are_unique() {
        let mut names: Vec<&str> = Step::ALL.iter().map(|s| s.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate step name");
    }

    #[test]
    fn every_implemented_step_is_declared() {
        for step in Step::IMPLEMENTED {
            assert!(Step::ALL.contains(step), "{step} missing from ALL");
        }
    }
}
