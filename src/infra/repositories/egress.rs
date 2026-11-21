//! Where each project's containers may connect outside its own network,
//! and what its names opened.
//!
//! A stored list that does not read back is read as empty: a project
//! whose list cannot be understood is closed, never opened.

use std::net::Ipv4Addr;

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use crate::domain::errors::PlatformResult;
use crate::domain::values::destinations::Destinations;

const REFUSED_KEPT: i64 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub name: String,
    pub times: i64,
    pub last_at: i64,
}

pub async fn of_project(
    database: &SqliteDatabase,
    project_id: &str,
) -> PlatformResult<Destinations> {
    let id = project_id.to_string();
    let text: Option<String> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"egress\" FROM project WHERE \"id\" = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()
        })
        .await?;
    Ok(text.map(|text| read(&text)).unwrap_or_default())
}

/// Replace the list, and close what a name the list no longer holds had
/// opened: an address is learned only for a line that allows it.
pub async fn set(
    database: &SqliteDatabase,
    project_id: &str,
    destinations: &Destinations,
) -> PlatformResult<()> {
    let id = project_id.to_string();
    let destinations = destinations.clone();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE project SET \"egress\" = ?2 WHERE \"id\" = ?1",
                (&id, destinations.text()),
            )?;
            let learned: Vec<(String, String, i64)> = connection
                .prepare(
                    "SELECT \"name\", \"address\", \"port\" FROM egress_learned \
                     WHERE \"project_id\" = ?1",
                )?
                .query_map([&id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect::<Result<_, _>>()?;
            for (name, address, port) in learned {
                if !destinations.ports_for(&name).contains(&port_of(port)) {
                    connection.execute(
                        "DELETE FROM egress_learned \
                         WHERE \"project_id\" = ?1 AND \"address\" = ?2 AND \"port\" = ?3",
                        (&id, address, port),
                    )?;
                }
            }
            Ok(())
        })
        .await?;
    Ok(())
}

/// A project with a network on this node, and what decides where its
/// containers go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enforced {
    pub index: u8,
    pub project_id: String,
    pub destinations: Destinations,
    pub exits: Vec<Ipv4Addr>,
}

pub async fn by_network(database: &SqliteDatabase) -> PlatformResult<Vec<Enforced>> {
    let rows: Vec<(i64, String, String, String)> = database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"network_index\", \"id\", \"egress\", \"exits\" FROM project \
                     WHERE \"network_index\" IS NOT NULL ORDER BY 1",
                )?
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?
                .collect()
        })
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(index, project_id, egress, exits)| {
            Some(Enforced {
                index: u8::try_from(index).ok()?,
                project_id,
                destinations: read(&egress),
                exits: addresses(&exits),
            })
        })
        .collect())
}

/// The overlay addresses of the nodes the project leaves through. Empty is
/// each copy through its own node.
pub async fn exits_of(
    database: &SqliteDatabase,
    project_id: &str,
) -> PlatformResult<Vec<Ipv4Addr>> {
    let id = project_id.to_string();
    let text: Option<String> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"exits\" FROM project WHERE \"id\" = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()
        })
        .await?;
    Ok(text.map(|text| addresses(&text)).unwrap_or_default())
}

pub async fn set_exits(
    database: &SqliteDatabase,
    project_id: &str,
    exits: &[Ipv4Addr],
) -> PlatformResult<()> {
    let id = project_id.to_string();
    let mut exits = exits.to_vec();
    exits.sort_unstable();
    exits.dedup();
    let text = exits
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE project SET \"exits\" = ?2 WHERE \"id\" = ?1",
                (id, text),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Open `addresses` for the project until `expires_at`, never shortening
/// what an earlier answer opened. Whether any of them was not open before,
/// which is when the kernel has to be told.
pub async fn learn(
    database: &SqliteDatabase,
    project_id: &str,
    name: &str,
    addresses: &[(Ipv4Addr, Option<u16>)],
    expires_at: i64,
    now: i64,
) -> PlatformResult<bool> {
    let id = project_id.to_string();
    let name = name.to_string();
    let addresses = addresses.to_vec();
    Ok(database
        .write(move |connection| {
            let mut opened = false;
            for (address, port) in addresses {
                let port = port.map(i64::from).unwrap_or(0);
                let held: Option<i64> = connection
                    .query_row(
                        "SELECT \"expires_at\" FROM egress_learned \
                         WHERE \"project_id\" = ?1 AND \"address\" = ?2 AND \"port\" = ?3",
                        (&id, address.to_string(), port),
                        |row| row.get(0),
                    )
                    .optional()?;
                opened |= held.is_none_or(|held| held <= now);
                connection.execute(
                    "INSERT INTO egress_learned \
                       (\"project_id\", \"address\", \"port\", \"name\", \"expires_at\") \
                     VALUES (?1, ?2, ?3, ?4, ?5) \
                     ON CONFLICT (\"project_id\", \"address\", \"port\") DO UPDATE SET \
                       \"name\" = excluded.\"name\", \
                       \"expires_at\" = MAX(\"expires_at\", excluded.\"expires_at\")",
                    (&id, address.to_string(), port, &name, expires_at),
                )?;
            }
            Ok(opened)
        })
        .await?)
}

/// What the project's names opened that is still in date, in the order
/// the kernel is told it.
pub async fn learned(
    database: &SqliteDatabase,
    project_id: &str,
    now: i64,
) -> PlatformResult<Vec<(Ipv4Addr, Option<u16>)>> {
    let id = project_id.to_string();
    let rows: Vec<(String, i64)> = database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"address\", \"port\" FROM egress_learned \
                     WHERE \"project_id\" = ?1 AND \"expires_at\" > ?2",
                )?
                .query_map((id, now), |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .await?;
    let mut learned: Vec<(Ipv4Addr, Option<u16>)> = rows
        .into_iter()
        .filter_map(|(address, port)| Some((address.parse().ok()?, port_of(port))))
        .collect();
    learned.sort_unstable();
    Ok(learned)
}

pub async fn forget_expired(database: &SqliteDatabase, now: i64) -> PlatformResult<usize> {
    Ok(database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM egress_learned WHERE \"expires_at\" <= ?1",
                [now],
            )
        })
        .await?)
}

pub async fn refuse(
    database: &SqliteDatabase,
    project_id: &str,
    name: &str,
    now: i64,
) -> PlatformResult<()> {
    let id = project_id.to_string();
    let name = name.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO egress_refused (\"project_id\", \"name\", \"times\", \"last_at\") \
                 VALUES (?1, ?2, 1, ?3) \
                 ON CONFLICT (\"project_id\", \"name\") DO UPDATE SET \
                   \"times\" = \"times\" + 1, \"last_at\" = excluded.\"last_at\"",
                (&id, &name, now),
            )?;
            connection.execute(
                "DELETE FROM egress_refused WHERE \"project_id\" = ?1 AND \"name\" NOT IN \
                   (SELECT \"name\" FROM egress_refused WHERE \"project_id\" = ?1 \
                    ORDER BY \"last_at\" DESC LIMIT ?2)",
                (&id, REFUSED_KEPT),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// The names the project asked for and was refused, latest first.
pub async fn refused(
    database: &SqliteDatabase,
    project_id: &str,
    limit: i64,
) -> PlatformResult<Vec<Refused>> {
    let id = project_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"name\", \"times\", \"last_at\" FROM egress_refused \
                     WHERE \"project_id\" = ?1 ORDER BY \"last_at\" DESC, \"name\" LIMIT ?2",
                )?
                .query_map((id, limit), |row| {
                    Ok(Refused {
                        name: row.get(0)?,
                        times: row.get(1)?,
                        last_at: row.get(2)?,
                    })
                })?
                .collect()
        })
        .await?)
}

fn read(text: &str) -> Destinations {
    Destinations::parse(text).unwrap_or_default()
}

fn addresses(text: &str) -> Vec<Ipv4Addr> {
    text.split(',')
        .filter_map(|address| address.trim().parse().ok())
        .collect()
}

fn port_of(stored: i64) -> Option<u16> {
    u16::try_from(stored).ok().filter(|port| *port != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::repositories::projects;

    #[tokio::test]
    async fn a_project_is_born_able_to_reach_anywhere() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "shop").await.expect("project");

        assert!(of_project(&database, &project.id)
            .await
            .expect("read")
            .is_anywhere());
    }

    #[tokio::test]
    async fn an_emptied_list_is_kept_empty_rather_than_read_as_the_default() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "shop").await.expect("project");

        set(&database, &project.id, &Destinations::default())
            .await
            .expect("set");

        assert!(of_project(&database, &project.id)
            .await
            .expect("read")
            .is_empty());
    }

    #[tokio::test]
    async fn only_a_project_with_a_network_here_has_a_list_to_enforce() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let networked = projects::create(&database, "shop").await.expect("project");
        projects::create(&database, "idle").await.expect("project");
        let index = projects::ensure_network_index(&database, &networked.id)
            .await
            .expect("index");
        let list = Destinations::parse("203.0.113.0/24:443").expect("parse");
        set(&database, &networked.id, &list).await.expect("set");

        assert_eq!(
            by_network(&database).await.expect("read"),
            vec![Enforced {
                index,
                project_id: networked.id.clone(),
                destinations: list,
                exits: Vec::new(),
            }]
        );
    }

    fn at(address: &str) -> Ipv4Addr {
        address.parse().expect("an address")
    }

    #[tokio::test]
    async fn an_address_stays_open_for_the_longest_answer_that_named_it() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "shop").await.expect("project");
        let opened = [(at("198.51.100.7"), Some(443))];

        assert!(learn(
            &database,
            &project.id,
            "api.example.com",
            &opened,
            5_000,
            1_000
        )
        .await
        .expect("learn"));
        assert!(
            !learn(
                &database,
                &project.id,
                "api.example.com",
                &opened,
                2_000,
                1_500
            )
            .await
            .expect("learn"),
            "already open"
        );
        assert_eq!(
            learned(&database, &project.id, 4_999).await.expect("read"),
            opened.to_vec(),
            "a shorter answer does not shorten it"
        );
        assert!(learned(&database, &project.id, 5_000)
            .await
            .expect("read")
            .is_empty());
        assert!(
            learn(
                &database,
                &project.id,
                "api.example.com",
                &opened,
                9_000,
                6_000
            )
            .await
            .expect("learn"),
            "an address that had run out is opened again"
        );
    }

    #[tokio::test]
    async fn a_name_taken_off_the_list_closes_what_it_opened() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "shop").await.expect("project");
        set(
            &database,
            &project.id,
            &Destinations::parse("api.example.com:443\n*.cdn.example.com").expect("a list"),
        )
        .await
        .expect("set");
        learn(
            &database,
            &project.id,
            "api.example.com",
            &[(at("198.51.100.7"), Some(443))],
            9_000,
            0,
        )
        .await
        .expect("learn");
        learn(
            &database,
            &project.id,
            "eu.cdn.example.com",
            &[(at("192.0.2.9"), None)],
            9_000,
            0,
        )
        .await
        .expect("learn");

        set(
            &database,
            &project.id,
            &Destinations::parse("*.cdn.example.com\napi.example.com:8443").expect("a list"),
        )
        .await
        .expect("set");

        assert_eq!(
            learned(&database, &project.id, 0).await.expect("read"),
            vec![(at("192.0.2.9"), None)]
        );
    }

    #[tokio::test]
    async fn a_refused_name_is_counted_and_only_the_latest_are_kept() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "shop").await.expect("project");
        for at in 0..60 {
            refuse(&database, &project.id, &format!("n{at}.example.com"), at)
                .await
                .expect("refuse");
        }
        refuse(&database, &project.id, "n59.example.com", 100)
            .await
            .expect("refuse");

        let all = refused(&database, &project.id, 1_000).await.expect("read");
        assert_eq!(all.len(), REFUSED_KEPT as usize);
        assert_eq!(
            all[0],
            Refused {
                name: "n59.example.com".into(),
                times: 2,
                last_at: 100
            }
        );
        assert!(!all.iter().any(|refused| refused.name == "n0.example.com"));
    }

    #[tokio::test]
    async fn a_project_leaves_through_its_own_nodes_until_it_names_exits() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "shop").await.expect("project");
        assert!(exits_of(&database, &project.id)
            .await
            .expect("read")
            .is_empty());

        set_exits(
            &database,
            &project.id,
            &[at("10.42.0.9"), at("10.42.0.2"), at("10.42.0.9")],
        )
        .await
        .expect("set");
        assert_eq!(
            exits_of(&database, &project.id).await.expect("read"),
            vec![at("10.42.0.2"), at("10.42.0.9")]
        );

        set_exits(&database, &project.id, &[]).await.expect("set");
        assert!(exits_of(&database, &project.id)
            .await
            .expect("read")
            .is_empty());
    }
}
