//! Every `SELECT`, `INSERT` and `UPDATE` a [`ServiceEvent`] needs.

use wabot::sqlite::rusqlite::{params, Connection, Row};
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service_event::{EventKind, ServiceEvent};
use crate::domain::errors::PlatformResult;
use crate::infra::clock::now_ms;

const COLUMNS: &str =
    "\"id\", \"service_id\", \"container\", \"kind\", \"at\", \"until\", \"code\", \"detail\"";

pub struct Happened<'a> {
    pub service_id: &'a str,
    pub container: Option<&'a str>,
    pub kind: EventKind,
    pub at: i64,
    pub code: Option<i64>,
    pub detail: Option<&'a str>,
}

impl<'a> Happened<'a> {
    pub fn now(service_id: &'a str, kind: EventKind) -> Self {
        Self {
            service_id,
            container: None,
            kind,
            at: now_ms(),
            code: None,
            detail: None,
        }
    }
}

pub(crate) fn insert(
    connection: &Connection,
    happened: &Happened,
) -> wabot::sqlite::rusqlite::Result<String> {
    let id = format!("ev-{}", wabot::prelude::password::generate(12));
    connection.execute(
        &format!("INSERT INTO service_event ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7)"),
        params![
            id,
            happened.service_id,
            happened.container,
            happened.kind.as_str(),
            happened.at,
            happened.code,
            happened.detail,
        ],
    )?;
    Ok(id)
}

pub async fn record(database: &SqliteDatabase, happened: Happened<'_>) -> PlatformResult<String> {
    let owned = (
        happened.service_id.to_string(),
        happened.container.map(str::to_string),
        happened.kind,
        happened.at,
        happened.code,
        happened.detail.map(str::to_string),
    );
    Ok(database
        .write(move |connection| {
            let (service_id, container, kind, at, code, detail) = &owned;
            insert(
                connection,
                &Happened {
                    service_id,
                    container: container.as_deref(),
                    kind: *kind,
                    at: *at,
                    code: *code,
                    detail: detail.as_deref(),
                },
            )
        })
        .await?)
}

/// Every event of a service before `to`. All of them rather than the
/// window's: a stop before the window still decides what inside it was
/// meant to be running, and a service's events are a few a day.
pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
    to: i64,
) -> PlatformResult<Vec<ServiceEvent>> {
    let id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM service_event \
                     WHERE \"service_id\" = ?1 AND \"at\" < ?2 ORDER BY \"at\", \"rowid\""
                ))?
                .query_map(params![id, to], decode)?
                .filter_map(Result::transpose)
                .collect()
        })
        .await?)
}

/// What happened to one copy on this node: the half of its story a far
/// owner asks for.
pub async fn of_container(
    database: &SqliteDatabase,
    container: &str,
    from: i64,
    to: i64,
) -> PlatformResult<Vec<ServiceEvent>> {
    let container = container.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM service_event \
                     WHERE \"container\" = ?1 AND \"at\" < ?3 AND COALESCE(\"until\", \"at\") >= ?2 \
                     ORDER BY \"at\", \"rowid\""
                ))?
                .query_map(params![container, from, to], decode)?
                .filter_map(Result::transpose)
                .collect()
        })
        .await?)
}

/// The one still open of its kind for this copy, if any.
pub async fn open(
    database: &SqliteDatabase,
    service_id: &str,
    container: &str,
    kind: EventKind,
) -> PlatformResult<Option<ServiceEvent>> {
    let (id, container) = (service_id.to_string(), container.to_string());
    Ok(database
        .read(move |connection| {
            connection
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM service_event \
                     WHERE \"service_id\" = ?1 AND \"container\" = ?2 AND \"kind\" = ?3 \
                       AND \"until\" IS NULL ORDER BY \"at\" DESC LIMIT 1"
                ))?
                .query_map(params![id, container, kind.as_str()], decode)?
                .filter_map(Result::transpose)
                .collect::<Result<Vec<_>, _>>()
        })
        .await?
        .into_iter()
        .next())
}

pub async fn close(database: &SqliteDatabase, id: &str, until: i64) -> PlatformResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service_event SET \"until\" = ?2 WHERE \"id\" = ?1 AND \"until\" IS NULL",
                params![id, until],
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Events another node wrote about a copy it runs, kept here under this
/// node's own service and their own ids, so reading them again only
/// updates what changed — a span that has since closed.
pub async fn keep(
    database: &SqliteDatabase,
    service_id: &str,
    events: Vec<ServiceEvent>,
) -> PlatformResult<()> {
    if events.is_empty() {
        return Ok(());
    }
    let service_id = service_id.to_string();
    database
        .write(move |connection| {
            for event in &events {
                connection.execute(
                    &format!("INSERT OR REPLACE INTO service_event ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"),
                    params![
                        event.id,
                        service_id,
                        event.container,
                        event.kind.as_str(),
                        event.at,
                        event.until,
                        event.code,
                        event.detail,
                    ],
                )?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn forget_before(database: &SqliteDatabase, moment: i64) -> PlatformResult<usize> {
    Ok(database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM service_event WHERE COALESCE(\"until\", \"at\") < ?1",
                [moment],
            )
        })
        .await?)
}

/// A kind this build does not know is skipped rather than refused: a node
/// that was downgraded keeps rows a newer one wrote.
fn decode(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Option<ServiceEvent>> {
    let Some(kind) = EventKind::parse(&row.get::<_, String>(3)?) else {
        return Ok(None);
    };
    Ok(Some(ServiceEvent {
        id: row.get(0)?,
        service_id: row.get(1)?,
        container: row.get(2)?,
        kind,
        at: row.get(4)?,
        until: row.get(5)?,
        code: row.get(6)?,
        detail: row.get(7)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::open_in_memory;

    async fn with_service() -> (SqliteDatabase, String) {
        let database = open_in_memory().await.unwrap();
        let project = crate::infra::repositories::projects::create(&database, "demo")
            .await
            .unwrap();
        let service = crate::infra::repositories::services::create(
            &database,
            &project.id,
            "db",
            "node.example/demo/db:latest",
            &[],
        )
        .await
        .unwrap();
        (database, service.id)
    }

    #[tokio::test]
    async fn an_open_event_is_found_until_it_is_closed() {
        let (database, service) = with_service().await;
        let id = record(
            &database,
            Happened {
                service_id: &service,
                container: Some("p.db.2"),
                kind: EventKind::Unfollowed,
                at: 1_000,
                code: None,
                detail: None,
            },
        )
        .await
        .unwrap();
        let found = open(&database, &service, "p.db.2", EventKind::Unfollowed)
            .await
            .unwrap();
        assert_eq!(found.map(|event| event.id), Some(id.clone()));
        close(&database, &id, 5_000).await.unwrap();
        assert!(open(&database, &service, "p.db.2", EventKind::Unfollowed)
            .await
            .unwrap()
            .is_none());
        let all = of_service(&database, &service, 10_000).await.unwrap();
        assert_eq!(all[0].until, Some(5_000));
        let mine = of_container(&database, "p.db.2", 4_000, 10_000)
            .await
            .unwrap();
        assert_eq!(
            mine.len(),
            1,
            "a span that reaches into the window is in it"
        );
    }
}
