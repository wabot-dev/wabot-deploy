//! Every `SELECT`, `INSERT` and `DELETE` an [`Allowlist`] needs.

use std::collections::HashMap;

use ipnet::IpNet;
use wabot::sqlite::{SqliteDatabase, SqliteResult};

use crate::domain::errors::PlatformResult;
use crate::domain::values::allowlist::{written, Allowlist, Door, Entry};
use crate::infra::clock::now_ms;

/// What `door` admits. No rows is an empty list, which admits nobody.
pub async fn of(database: &SqliteDatabase, door: &Door) -> PlatformResult<Allowlist> {
    let (kind, subject) = door.columns();
    let (kind, subject) = (kind.to_string(), subject.to_string());
    let rows: Vec<(String, Option<String>)> = database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"network\", \"note\" FROM allowlist \
                     WHERE \"door\" = ?1 AND \"subject\" = ?2 ORDER BY \"created_at\", \"rowid\"",
                )?
                .query_map((kind, subject), |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .await?;
    Ok(Allowlist::new(
        rows.into_iter()
            .filter_map(|(network, note)| {
                network
                    .parse::<IpNet>()
                    .ok()
                    .map(|network| Entry { network, note })
            })
            .collect(),
    ))
}

/// Replace what `door` admits with `list`, whole.
pub async fn set(database: &SqliteDatabase, door: &Door, list: &Allowlist) -> PlatformResult<()> {
    let (kind, subject) = door.columns();
    let (kind, subject) = (kind.to_string(), subject.to_string());
    let rows: Vec<(String, Option<String>)> = list
        .entries()
        .iter()
        .map(|entry| (entry.network.to_string(), entry.note.clone()))
        .collect();
    let now = now_ms();
    database
        .write(move |connection| {
            // One transaction, so a reload between the delete and the
            // inserts cannot read the door as closed.
            let transaction = connection.unchecked_transaction()?;
            transaction.execute(
                "DELETE FROM allowlist WHERE \"door\" = ?1 AND \"subject\" = ?2",
                (&kind, &subject),
            )?;
            for (network, note) in rows {
                transaction.execute(
                    "INSERT INTO allowlist (\"door\", \"subject\", \"network\", \"note\", \"created_at\") \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    (&kind, &subject, network, note, now),
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Every door's ranges at once, for the edge to load in one read.
pub async fn all(database: &SqliteDatabase) -> SqliteResult<HashMap<Door, Vec<IpNet>>> {
    let rows: Vec<(String, String, String)> = database
        .read(|connection| {
            connection
                .prepare("SELECT \"door\", \"subject\", \"network\" FROM allowlist")?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect()
        })
        .await?;
    let mut doors: HashMap<Door, Vec<IpNet>> = HashMap::new();
    for (kind, subject, network) in rows {
        let (Some(door), Ok(network)) = (
            Door::from_columns(&kind, &subject),
            network.parse::<IpNet>(),
        ) else {
            continue;
        };
        doors.entry(door).or_default().push(network);
    }
    Ok(doors)
}

/// How a list travels to another node and is kept on its route: the
/// ranges, comma-separated, and `""` for nobody.
pub fn carried(networks: &[IpNet]) -> String {
    networks.iter().map(written).collect::<Vec<_>>().join(",")
}

/// The other way. A part that will not parse is dropped, so a newer
/// owner's spelling narrows the list rather than opening it.
pub fn uncarried(text: &str) -> Vec<IpNet> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .filter_map(|part| match part.contains('/') {
            true => part.parse::<IpNet>().ok(),
            false => part.parse::<std::net::IpAddr>().ok().map(IpNet::from),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> SqliteDatabase {
        crate::infra::db::open_in_memory().await.expect("open")
    }

    /// The migration's promise: a node that existed before allowlists
    /// opens its own doors to everyone, so the console it is upgraded
    /// from is still there afterwards.
    #[tokio::test]
    async fn a_node_starts_with_its_console_and_registry_open_to_everyone() {
        let database = database().await;
        for door in [Door::Console, Door::Registry] {
            assert!(
                of(&database, &door).await.expect("read").is_everyone(),
                "{door:?}"
            );
        }
    }

    /// A project takes its services by cascade, which no caller sees: a
    /// delete beside each caller would have left their lists behind, to be
    /// read by whatever reused the id.
    #[tokio::test]
    async fn a_list_goes_with_what_it_belongs_to_even_by_cascade() {
        let database = database().await;
        let project = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &database,
            &project.id,
            "api",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("service");
        let list = Allowlist::parse("203.0.113.5").expect("parsed");
        for door in [
            Door::Service(service.id.clone()),
            Door::ProjectRegistry(project.id.clone()),
        ] {
            set(&database, &door, &list).await.expect("set");
        }

        crate::infra::repositories::projects::delete(&database, &project.id)
            .await
            .expect("delete");

        let left = all(&database).await.expect("all");
        assert_eq!(
            left.keys()
                .filter(|door| !door.columns().1.is_empty())
                .count(),
            0,
            "{left:?}"
        );
    }

    #[tokio::test]
    async fn a_door_nobody_wrote_for_is_closed() {
        let database = database().await;
        let list = of(&database, &Door::Service("svc-new".into()))
            .await
            .expect("read");
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn a_list_is_replaced_whole_and_kept_in_order() {
        let database = database().await;
        let door = Door::Service("svc-1".into());
        let first = Allowlist::parse("203.0.113.5 office\n10.8.0.0/16 vpn").expect("parsed");
        set(&database, &door, &first).await.expect("set");
        assert_eq!(of(&database, &door).await.expect("read"), first);

        let second = Allowlist::parse("198.51.100.0/24").expect("parsed");
        set(&database, &door, &second).await.expect("set");
        assert_eq!(of(&database, &door).await.expect("read"), second);

        set(&database, &door, &Allowlist::default())
            .await
            .expect("set");
        assert!(of(&database, &door).await.expect("read").is_empty());
        assert!(
            !all(&database).await.expect("all").contains_key(&door),
            "and closed is no rows, not a row that says so"
        );
    }

    #[test]
    fn a_list_survives_travelling_and_nobody_is_an_empty_string() {
        let list = Allowlist::parse("203.0.113.5\n2001:db8::/32").expect("parsed");
        let carried_list = carried(&list.networks());
        assert_eq!(carried_list, "203.0.113.5,2001:db8::/32");
        assert_eq!(uncarried(&carried_list), list.networks());
        assert_eq!(carried(&[]), "");
        assert!(uncarried("").is_empty());
    }
}
