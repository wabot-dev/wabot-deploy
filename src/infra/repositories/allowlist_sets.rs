use ipnet::IpNet;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::allowlist_set::AllowlistSet;
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::domain::values::allowlist::{Allowlist, Entry};
use crate::infra::clock::now_ms;

pub const ALREADY_NAMED: &str = "this project already has a list by that name";

pub async fn of_project(
    database: &SqliteDatabase,
    project_id: &str,
) -> PlatformResult<Vec<AllowlistSet>> {
    let project_id = project_id.to_string();
    let heads: Vec<(String, String, String)> = database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"project_id\", \"name\" FROM allowlist_set \
                     WHERE \"project_id\" = ?1 ORDER BY \"name\"",
                )?
                .query_map([project_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect()
        })
        .await?;
    filled(database, heads).await
}

pub async fn find(
    database: &SqliteDatabase,
    project_id: &str,
    name: &str,
) -> PlatformResult<Option<AllowlistSet>> {
    Ok(of_project(database, project_id)
        .await?
        .into_iter()
        .find(|set| set.name == name))
}

pub async fn create(
    database: &SqliteDatabase,
    project_id: &str,
    name: &str,
    list: &Allowlist,
) -> PlatformResult<AllowlistSet> {
    let set = AllowlistSet {
        id: format!("als-{}", wabot::prelude::password::generate(12)),
        project_id: project_id.to_string(),
        name: name.to_string(),
        list: list.clone(),
    };
    let (id, project, name, rows) = (
        set.id.clone(),
        set.project_id.clone(),
        set.name.clone(),
        rows_of(list),
    );
    database
        .transaction(move |transaction| {
            let now = now_ms();
            transaction.execute(
                "INSERT INTO allowlist_set (\"id\", \"project_id\", \"name\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4)",
                (&id, project, name, now),
            )?;
            insert_rows(transaction, &id, rows, now)
        })
        .await
        .map_err(|error| match error.to_string().contains("UNIQUE") {
            true => PlatformError::Refused(ALREADY_NAMED.into()),
            false => PlatformError::Storage(error),
        })?;
    Ok(set)
}

/// One transaction, so a reload between the delete and the inserts cannot
/// read every service using it as closed to the list's addresses.
pub async fn set_list(
    database: &SqliteDatabase,
    set_id: &str,
    list: &Allowlist,
) -> PlatformResult<()> {
    let (id, rows) = (set_id.to_string(), rows_of(list));
    database
        .transaction(move |transaction| {
            transaction.execute(
                "DELETE FROM allowlist_set_entry WHERE \"set_id\" = ?1",
                [&id],
            )?;
            insert_rows(transaction, &id, rows, now_ms())
        })
        .await?;
    Ok(())
}

pub async fn delete(database: &SqliteDatabase, set_id: &str) -> PlatformResult<()> {
    let id = set_id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM allowlist_set WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Vec<AllowlistSet>> {
    let service_id = service_id.to_string();
    let heads: Vec<(String, String, String)> = database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT s.\"id\", s.\"project_id\", s.\"name\" FROM allowlist_set s \
                     JOIN service_allowlist_set u ON u.\"set_id\" = s.\"id\" \
                     WHERE u.\"service_id\" = ?1 ORDER BY s.\"name\"",
                )?
                .query_map([service_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect()
        })
        .await?;
    filled(database, heads).await
}

pub async fn set_for_service(
    database: &SqliteDatabase,
    service_id: &str,
    set_ids: &[String],
) -> PlatformResult<()> {
    let (service_id, set_ids) = (service_id.to_string(), set_ids.to_vec());
    database
        .transaction(move |transaction| {
            transaction.execute(
                "DELETE FROM service_allowlist_set WHERE \"service_id\" = ?1",
                [&service_id],
            )?;
            for set_id in set_ids {
                transaction.execute(
                    "INSERT OR IGNORE INTO service_allowlist_set (\"service_id\", \"set_id\") \
                     VALUES (?1, ?2)",
                    (&service_id, set_id),
                )?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn services_using(
    database: &SqliteDatabase,
    set_id: &str,
) -> PlatformResult<Vec<String>> {
    let set_id = set_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"service_id\" FROM service_allowlist_set WHERE \"set_id\" = ?1 \
                     ORDER BY \"service_id\"",
                )?
                .query_map([set_id], |row| row.get(0))?
                .collect()
        })
        .await?)
}

async fn filled(
    database: &SqliteDatabase,
    heads: Vec<(String, String, String)>,
) -> PlatformResult<Vec<AllowlistSet>> {
    let ids: Vec<String> = heads.iter().map(|(id, _, _)| id.clone()).collect();
    let entries: Vec<(String, String, Option<String>)> = database
        .read(move |connection| {
            let mut statement = connection.prepare(
                "SELECT \"set_id\", \"network\", \"note\" FROM allowlist_set_entry \
                 ORDER BY \"created_at\", \"rowid\"",
            )?;
            let rows = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<Vec<(String, String, Option<String>)>, _>>()?;
            Ok(rows
                .into_iter()
                .filter(|(set_id, _, _)| ids.contains(set_id))
                .collect())
        })
        .await?;
    Ok(heads
        .into_iter()
        .map(|(id, project_id, name)| {
            let list = Allowlist::new(
                entries
                    .iter()
                    .filter(|(set_id, _, _)| *set_id == id)
                    .filter_map(|(_, network, note)| {
                        network.parse::<IpNet>().ok().map(|network| Entry {
                            network,
                            note: note.clone(),
                        })
                    })
                    .collect(),
            );
            AllowlistSet {
                id,
                project_id,
                name,
                list,
            }
        })
        .collect())
}

fn rows_of(list: &Allowlist) -> Vec<(String, Option<String>)> {
    list.entries()
        .iter()
        .map(|entry| (entry.network.to_string(), entry.note.clone()))
        .collect()
}

fn insert_rows(
    transaction: &wabot::sqlite::rusqlite::Transaction<'_>,
    set_id: &str,
    rows: Vec<(String, Option<String>)>,
    now: i64,
) -> wabot::sqlite::rusqlite::Result<()> {
    for (network, note) in rows {
        transaction.execute(
            "INSERT INTO allowlist_set_entry (\"set_id\", \"network\", \"note\", \"created_at\") \
             VALUES (?1, ?2, ?3, ?4)",
            (set_id, network, note, now),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_list_is_kept_with_its_notes_and_replaced_whole() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");
        let office = Allowlist::parse("203.0.113.5 office\n10.8.0.0/16 vpn").expect("parsed");
        let made = create(&database, &project.id, "office", &office)
            .await
            .expect("made");
        assert_eq!(
            find(&database, &project.id, "office").await.expect("read"),
            Some(made.clone())
        );

        let moved = Allowlist::parse("198.51.100.0/24 new office").expect("parsed");
        set_list(&database, &made.id, &moved).await.expect("saved");
        assert_eq!(
            find(&database, &project.id, "office")
                .await
                .expect("read")
                .expect("there")
                .list,
            moved
        );

        let again = create(&database, &project.id, "office", &Allowlist::default()).await;
        assert!(
            matches!(&again, Err(PlatformError::Refused(reason)) if reason == ALREADY_NAMED),
            "{again:?}"
        );
    }
}
