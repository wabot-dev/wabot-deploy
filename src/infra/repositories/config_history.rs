//! Every `SELECT` and `INSERT` a [`Revision`] needs, and the depth the
//! history is kept to.

use std::collections::BTreeMap;

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::config_revision::Revision;
use crate::domain::errors::PlatformResult;
use crate::infra::clock::now_ms;

/// How many revisions a service keeps.
const KEEP: usize = 20;
/// Record the environment a service is being given.
///
/// Called with the values *about* to take effect, so the newest
/// revision always describes what is running. A history that records
/// the previous value instead is one where the top row is a lie.
pub async fn record(
    database: &SqliteDatabase,
    service_id: &str,
    env: &BTreeMap<String, String>,
    changed_by: Option<&str>,
    reason: &str,
) -> PlatformResult<()> {
    let (service, env, by, reason) = (
        service_id.to_string(),
        env.clone(),
        changed_by.map(str::to_string),
        reason.to_string(),
    );
    database
        .write(move |connection| insert(connection, &service, &env, by.as_deref(), &reason))
        .await?;
    Ok(())
}

/// The same, inside a write somebody else is making — so a service and
/// the revision describing it are one transaction.
pub(crate) fn insert(
    connection: &wabot::sqlite::rusqlite::Connection,
    service_id: &str,
    env: &BTreeMap<String, String>,
    changed_by: Option<&str>,
    reason: &str,
) -> wabot::sqlite::rusqlite::Result<()> {
    let id = format!("cfg-{}", wabot::prelude::password::generate(12));
    let payload = serde_json::to_string(env).unwrap_or_else(|_| "{}".into());
    connection.execute(
        "INSERT INTO config_revision \
           (\"id\", \"service_id\", \"env\", \"changed_by\", \"reason\", \"created_at\") \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (id, service_id, payload, changed_by, reason, now_ms()),
    )?;
    // Pruned here rather than on a timer: this is the only place that
    // adds one.
    connection.execute(
        "DELETE FROM config_revision WHERE \"service_id\" = ?1 AND \"id\" NOT IN \
           (SELECT \"id\" FROM config_revision WHERE \"service_id\" = ?1 \
            ORDER BY \"created_at\" DESC, \"rowid\" DESC LIMIT ?2)",
        (service_id, KEEP as i64),
    )?;
    Ok(())
}

/// A service's history, newest first.
pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Vec<Revision>> {
    let id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"service_id\", \"env\", \"changed_by\", \"reason\", \
                     \"created_at\" FROM config_revision WHERE \"service_id\" = ?1 \
                     ORDER BY \"created_at\" DESC, \"rowid\" DESC",
                )?
                .query_map([id], decode)?
                .collect()
        })
        .await?)
}

pub async fn find(database: &SqliteDatabase, id: &str) -> PlatformResult<Option<Revision>> {
    let id = id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"service_id\", \"env\", \"changed_by\", \"reason\", \
                     \"created_at\" FROM config_revision WHERE \"id\" = ?1",
                )?
                .query_map([id], decode)?
                .collect::<Result<Vec<_>, _>>()
        })
        .await?
        .into_iter()
        .next())
}

fn decode(row: &wabot::sqlite::rusqlite::Row<'_>) -> wabot::sqlite::rusqlite::Result<Revision> {
    let env: String = row.get(2)?;
    Ok(Revision {
        id: row.get(0)?,
        service_id: row.get(1)?,
        // A row we cannot parse still lists, without its values,
        // rather than taking the page down.
        env: serde_json::from_str(&env).unwrap_or_default(),
        changed_by: row.get(3)?,
        reason: row.get(4)?,
        created_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::repositories::services;

    async fn service() -> (SqliteDatabase, String) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &database,
            &project.id,
            "api",
            "nginx:alpine",
            &[],
        )
        .await
        .expect("service");
        (database, service.id)
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn the_newest_revision_describes_what_is_running() {
        let (database, service) = service().await;
        record(&database, &service, &env(&[("LOG", "info")]), None, "edit")
            .await
            .expect("recorded");
        record(&database, &service, &env(&[("LOG", "debug")]), None, "edit")
            .await
            .expect("recorded");

        let history = of_service(&database, &service).await.expect("history");
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].env.get("LOG").map(String::as_str), Some("debug"));
        assert_eq!(history[1].env.get("LOG").map(String::as_str), Some("info"));
        assert_eq!(history[2].reason, "create");
    }

    /// The values a service was made with are a revision too, or the
    /// first edit is one nobody can take back.
    ///
    /// **Shipped otherwise**: creating a service recorded nothing, so
    /// after one edit the history held only what was running and offered
    /// nothing to restore. Found on a VM node.
    #[tokio::test]
    async fn a_service_is_made_with_its_first_revision() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        let made = services::create(
            &database,
            &project.id,
            "api",
            "nginx:alpine",
            &[("GREETING".to_string(), "hello".to_string())],
        )
        .await
        .expect("service");

        let history = of_service(&database, &made.id).await.expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].reason, "create");
        assert_eq!(
            history[0].env.get("GREETING").map(String::as_str),
            Some("hello")
        );
    }

    /// A history nobody prunes is a table nobody reads.
    #[tokio::test]
    async fn only_the_last_twenty_are_kept() {
        let (database, service) = service().await;
        for n in 0..25 {
            record(
                &database,
                &service,
                &env(&[("N", &n.to_string())]),
                None,
                "edit",
            )
            .await
            .expect("recorded");
        }

        let history = of_service(&database, &service).await.expect("history");
        assert_eq!(history.len(), KEEP);
        assert_eq!(history[0].env.get("N").map(String::as_str), Some("24"));
        assert_eq!(
            history[KEEP - 1].env.get("N").map(String::as_str),
            Some("5"),
            "the oldest five were dropped"
        );
    }

    /// What the whole thing is for: going back to a set of values
    /// without touching which image is running.
    #[tokio::test]
    async fn an_older_revision_can_be_read_back() {
        let (database, service) = service().await;
        record(&database, &service, &env(&[("KEY", "one")]), None, "edit")
            .await
            .expect("recorded");
        let first = of_service(&database, &service)
            .await
            .expect("history")
            .into_iter()
            .next()
            .expect("one");

        record(&database, &service, &env(&[("KEY", "two")]), None, "edit")
            .await
            .expect("recorded");

        let found = find(&database, &first.id)
            .await
            .expect("find")
            .expect("present");
        assert_eq!(found.env.get("KEY").map(String::as_str), Some("one"));
    }

    #[tokio::test]
    async fn a_revision_says_what_it_was() {
        let (database, service) = service().await;
        record(&database, &service, &env(&[]), None, "revert")
            .await
            .expect("recorded");

        let history = of_service(&database, &service).await.expect("history");
        assert_eq!(history[0].reason, "revert");
    }

    #[tokio::test]
    async fn revisions_go_with_the_service() {
        let (database, service) = service().await;
        record(&database, &service, &env(&[("A", "1")]), None, "edit")
            .await
            .expect("recorded");

        services::delete(&database, &service).await.expect("delete");
        assert!(of_service(&database, &service)
            .await
            .expect("history")
            .is_empty());
    }
}
