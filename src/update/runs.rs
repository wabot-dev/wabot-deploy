//! The record of an update attempt.
//!
//! Kept in the database rather than in memory because the last step of
//! an update is the process being replaced. A run marked `restarting`
//! is a question left for whoever comes back — see
//! [`super::settle_after_restart`].

use serde::Serialize;
use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::{SqliteDatabase, SqliteResult};

use crate::platform::now_ms;

/// Where an update got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    /// Downloading, verifying, swapping.
    Running,
    /// The binary is in place and the service was asked to restart.
    /// Nothing more will be written by *this* process.
    Restarting,
    Done,
    Failed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Restarting => "restarting",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "running" => Self::Running,
            "restarting" => Self::Restarting,
            "done" => Self::Done,
            // An unknown value is not a run in progress: treating it as
            // one would block every future update on a row nobody can
            // interpret.
            _ => Self::Failed,
        }
    }

    /// Is something still happening?
    pub fn in_flight(self) -> bool {
        matches!(self, Self::Running | Self::Restarting)
    }
}

#[derive(Debug, Clone)]
pub struct Run {
    pub id: String,
    pub from_version: String,
    pub to_version: String,
    pub tag: String,
    pub status: Status,
    pub step: Option<String>,
    pub detail: Option<String>,
    pub backup_path: Option<String>,
    pub account_id: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

const COLUMNS: &str = "\"id\", \"from_version\", \"to_version\", \"tag\", \"status\", \"step\", \
                       \"detail\", \"backup_path\", \"account_id\", \"started_at\", \"finished_at\"";

fn read(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Run> {
    Ok(Run {
        id: row.get(0)?,
        from_version: row.get(1)?,
        to_version: row.get(2)?,
        tag: row.get(3)?,
        status: Status::parse(&row.get::<_, String>(4)?),
        step: row.get(5)?,
        detail: row.get(6)?,
        backup_path: row.get(7)?,
        account_id: row.get(8)?,
        started_at: row.get(9)?,
        finished_at: row.get(10)?,
    })
}

pub async fn start(
    database: &SqliteDatabase,
    from_version: &str,
    to_version: &str,
    tag: &str,
    account_id: Option<&str>,
) -> SqliteResult<Run> {
    let run = Run {
        id: format!("upd-{}", wabot::prelude::password::generate(12)),
        from_version: from_version.to_string(),
        to_version: to_version.to_string(),
        tag: tag.to_string(),
        status: Status::Running,
        step: Some("starting".into()),
        detail: None,
        backup_path: None,
        account_id: account_id.map(str::to_string),
        started_at: now_ms(),
        finished_at: None,
    };

    let insert = run.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO update_run \
                   (\"id\", \"from_version\", \"to_version\", \"tag\", \"status\", \"step\", \
                    \"account_id\", \"started_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                (
                    insert.id,
                    insert.from_version,
                    insert.to_version,
                    insert.tag,
                    insert.status.as_str(),
                    insert.step,
                    insert.account_id,
                    insert.started_at,
                ),
            )?;
            Ok(())
        })
        .await?;
    Ok(run)
}

/// Say what is happening now. Only for a run still in flight — the
/// step of a finished run is history.
pub async fn set_step(database: &SqliteDatabase, id: &str, step: &str) -> SqliteResult<()> {
    let (id, step) = (id.to_string(), step.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE update_run SET \"step\" = ?2 WHERE \"id\" = ?1",
                (id, step),
            )?;
            Ok(())
        })
        .await
}

pub async fn set_backup(database: &SqliteDatabase, id: &str, path: &str) -> SqliteResult<()> {
    let (id, path) = (id.to_string(), path.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE update_run SET \"backup_path\" = ?2 WHERE \"id\" = ?1",
                (id, path),
            )?;
            Ok(())
        })
        .await
}

/// Move a run to its next state.
///
/// `finished_at` is set for the states that end it, so "how long did
/// this take" is answerable and a `restarting` row can be recognised
/// as unfinished by the process that comes back.
pub async fn finish(
    database: &SqliteDatabase,
    id: &str,
    status: Status,
    detail: Option<&str>,
) -> SqliteResult<()> {
    let (id, detail) = (id.to_string(), detail.map(str::to_string));
    let finished = match status {
        Status::Done | Status::Failed => Some(now_ms()),
        _ => None,
    };
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE update_run SET \"status\" = ?2, \"detail\" = ?3, \"finished_at\" = ?4 \
                 WHERE \"id\" = ?1",
                (id, status.as_str(), detail, finished),
            )?;
            Ok(())
        })
        .await
}

/// The most recent attempt, whatever became of it.
pub async fn latest(database: &SqliteDatabase) -> SqliteResult<Option<Run>> {
    database
        .read(move |connection| {
            connection
                .query_row(
                    &format!(
                        "SELECT {COLUMNS} FROM update_run \
                         ORDER BY \"started_at\" DESC, rowid DESC LIMIT 1"
                    ),
                    [],
                    read,
                )
                .optional()
        })
        .await
}

/// The last few, for the page that shows what this node has installed.
pub async fn recent(database: &SqliteDatabase, limit: usize) -> SqliteResult<Vec<Run>> {
    database
        .read(move |connection| {
            connection
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM update_run \
                     ORDER BY \"started_at\" DESC, rowid DESC LIMIT ?1"
                ))?
                .query_map([limit as i64], read)?
                .collect()
        })
        .await
}

/// Is an update already happening?
///
/// A stale `running` row — from a process killed between the download
/// and the swap — would otherwise block every future update, so age is
/// part of the answer.
pub async fn in_flight(database: &SqliteDatabase) -> SqliteResult<Option<Run>> {
    const STALE_AFTER: i64 = 20 * 60 * 1000;
    Ok(latest(database)
        .await?
        .filter(|run| run.status.in_flight() && now_ms() - run.started_at < STALE_AFTER))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> SqliteDatabase {
        crate::db::open_in_memory().await.expect("open")
    }

    #[tokio::test]
    async fn a_run_is_recorded_and_settled() {
        let database = db().await;
        let run = start(&database, "0.1.0", "0.2.0", "v0.2.0", None)
            .await
            .expect("start");

        assert_eq!(
            in_flight(&database).await.expect("read").map(|r| r.id),
            Some(run.id.clone()),
            "a running update is in flight"
        );

        set_step(&database, &run.id, "downloading")
            .await
            .expect("step");
        finish(&database, &run.id, Status::Done, Some("installed"))
            .await
            .expect("finish");

        let stored = latest(&database).await.expect("read").expect("a run");
        assert_eq!(stored.status, Status::Done);
        assert_eq!(stored.step.as_deref(), Some("downloading"));
        assert_eq!(stored.detail.as_deref(), Some("installed"));
        assert!(stored.finished_at.is_some());
        assert!(in_flight(&database).await.expect("read").is_none());
    }

    /// A process killed mid-update leaves a `running` row nobody will
    /// ever finish. Blocking updates forever on it would make the
    /// recovery "edit the database".
    #[tokio::test]
    async fn a_stale_run_does_not_block_the_next_one() {
        let database = db().await;
        let run = start(&database, "0.1.0", "0.2.0", "v0.2.0", None)
            .await
            .expect("start");

        let id = run.id.clone();
        let long_ago = now_ms() - 60 * 60 * 1000;
        database
            .write(move |connection| {
                connection.execute(
                    "UPDATE update_run SET \"started_at\" = ?2 WHERE \"id\" = ?1",
                    (id, long_ago),
                )?;
                Ok(())
            })
            .await
            .expect("age it");

        assert!(in_flight(&database).await.expect("read").is_none());
    }

    #[tokio::test]
    async fn the_newest_run_is_the_latest_even_within_a_millisecond() {
        let database = db().await;
        start(&database, "0.1.0", "0.2.0", "v0.2.0", None)
            .await
            .expect("start");
        let second = start(&database, "0.2.0", "0.3.0", "v0.3.0", None)
            .await
            .expect("start");

        assert_eq!(
            latest(&database).await.expect("read").map(|r| r.id),
            Some(second.id)
        );
        assert_eq!(recent(&database, 10).await.expect("read").len(), 2);
    }
}
