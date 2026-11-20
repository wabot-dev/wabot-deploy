//! The addresses names with no copy on this node resolve to.

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use crate::domain::errors::{PlatformError, PlatformResult};

/// The host part, in `10.42.255.0/24`, that `name` stands in at for this
/// project: the one it already has, or the lowest free.
pub async fn ensure(database: &SqliteDatabase, project_id: &str, name: &str) -> PlatformResult<u8> {
    let (project, name) = (project_id.to_string(), name.to_string());
    let host: Option<i64> = database
        .write(move |connection| {
            let existing: Option<i64> = connection
                .query_row(
                    "SELECT \"host\" FROM stand_in WHERE \"project_id\" = ?1 AND \"name\" = ?2",
                    (&project, &name),
                    |row| row.get(0),
                )
                .optional()?;
            if existing.is_some() {
                return Ok(existing);
            }
            let taken: Vec<i64> = connection
                .prepare("SELECT \"host\" FROM stand_in WHERE \"project_id\" = ?1")?
                .query_map([&project], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            let Some(free) = (1..=254i64).find(|host| !taken.contains(host)) else {
                return Ok(None);
            };
            connection.execute(
                "INSERT INTO stand_in (\"project_id\", \"name\", \"host\") VALUES (?1, ?2, ?3)",
                (&project, &name, free),
            )?;
            Ok(Some(free))
        })
        .await?;
    host.map(|host| host as u8).ok_or_else(|| {
        PlatformError::Refused("every stand-in address of this project is in use".into())
    })
}

/// The name standing in at `host` in this project.
pub async fn name_at(
    database: &SqliteDatabase,
    project_id: &str,
    host: u8,
) -> PlatformResult<Option<String>> {
    let project = project_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"name\" FROM stand_in WHERE \"project_id\" = ?1 AND \"host\" = ?2",
                    (project, i64::from(host)),
                    |row| row.get(0),
                )
                .optional()
        })
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_name_keeps_its_address_and_the_next_takes_the_lowest_free() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let shop = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");
        let other = crate::infra::repositories::projects::create(&database, "other")
            .await
            .expect("project");

        assert_eq!(ensure(&database, &shop.id, "orders").await.expect("a"), 1);
        assert_eq!(
            ensure(&database, &shop.id, "orders-ro").await.expect("b"),
            2
        );
        assert_eq!(
            ensure(&database, &shop.id, "orders").await.expect("kept"),
            1
        );
        assert_eq!(
            ensure(&database, &other.id, "web").await.expect("its own"),
            1,
            "each project's bridge is its own, so the addresses are too"
        );
        assert_eq!(
            name_at(&database, &shop.id, 2)
                .await
                .expect("read")
                .as_deref(),
            Some("orders-ro")
        );
        assert_eq!(name_at(&database, &shop.id, 3).await.expect("read"), None);
    }
}
