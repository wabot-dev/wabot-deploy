//! Which nodes answer for which of a service's names, and the secret a
//! co-edge presents.

use wabot::sqlite::SqliteDatabase;

use crate::clock::now_ms;
use crate::errors::PlatformResult;

pub async fn set(
    database: &SqliteDatabase,
    service_id: &str,
    hostname: &str,
    nodes: &[String],
) -> PlatformResult<Vec<String>> {
    let before = nodes_for(database, hostname).await?;

    let (service_id, host, wanted) = (service_id.to_string(), hostname.to_string(), nodes.to_vec());
    database
        .write(move |connection| {
            connection.execute("DELETE FROM service_edge WHERE \"hostname\" = ?1", [&host])?;
            for node in &wanted {
                connection.execute(
                    "INSERT INTO service_edge \
                       (\"service_id\", \"hostname\", \"node_id\", \"created_at\") \
                     VALUES (?1, ?2, ?3, ?4)",
                    (&service_id, &host, node, now_ms()),
                )?;
            }
            Ok(())
        })
        .await?;

    // The relay rows follow the set, here rather than in the caller: a
    // name whose edges changed and whose peer list did not is a node
    // publishing a challenge to somebody who is no longer answering, and
    // — worse — not publishing it to somebody who is. Written even for a
    // single node, because the secret has to exist before the first
    // errand carries it.
    remember_peers(database, hostname, nodes).await?;

    Ok(before
        .into_iter()
        .filter(|node| !nodes.contains(node))
        .collect())
}

/// Who answers for this name.
pub async fn nodes_for(database: &SqliteDatabase, hostname: &str) -> PlatformResult<Vec<String>> {
    let hostname = hostname.to_string();
    Ok(database
        .read(move |connection| {
            let mut statement = connection.prepare(
                "SELECT \"node_id\" FROM service_edge \
                 WHERE \"hostname\" = ?1 ORDER BY \"created_at\"",
            )?;
            let nodes: wabot::sqlite::rusqlite::Result<Vec<String>> =
                statement.query_map([hostname], |row| row.get(0))?.collect();
            nodes
        })
        .await?)
}

/// Every name one node was chosen to answer for.
///
/// `None` for the node id — a node with no row of its own yet — answers
/// for nothing, which is the honest reading: a node that does not know
/// who it is cannot have been chosen.
pub async fn of_node(
    database: &SqliteDatabase,
    node_id: Option<&str>,
) -> PlatformResult<Vec<String>> {
    let Some(node_id) = node_id.map(str::to_string) else {
        return Ok(Vec::new());
    };
    Ok(database
        .read(move |connection| {
            let mut statement = connection
                .prepare("SELECT \"hostname\" FROM service_edge WHERE \"node_id\" = ?1")?;
            let names: wabot::sqlite::rusqlite::Result<Vec<String>> =
                statement.query_map([node_id], |row| row.get(0))?.collect();
            names
        })
        .await?)
}

/// Every (hostname, node) this service has asked for.
pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Vec<(String, String)>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            let mut statement = connection.prepare(
                "SELECT \"hostname\", \"node_id\" FROM service_edge \
                 WHERE \"service_id\" = ?1 ORDER BY \"hostname\", \"created_at\"",
            )?;
            let rows: wabot::sqlite::rusqlite::Result<Vec<(String, String)>> = statement
                .query_map([service_id], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect();
            rows
        })
        .await?)
}

// ---------- the challenge relay -------------------------------------------
//
// Who else answers for a name, and the secret they reach each other with.
// See migration `0045`: an HTTP-01 token lives in the ordering node's own
// table, and a name several nodes answer for is one the authority may ask
// the wrong node for.

/// Record the set for this name, and mint the secret if there is none.
///
/// **Convergent**: the peers are replaced whole, and the secret is left
/// alone once it exists. Rotating it on every save would invalidate the
/// copy every other edge is holding, which is the one thing that must not
/// happen to a shared secret nobody re-reads.
pub async fn remember_peers(
    database: &SqliteDatabase,
    hostname: &str,
    nodes: &[String],
) -> PlatformResult<()> {
    // **This node is not one of its own peers.** What reads these rows is
    // the publish, and a node dialling itself over the overlay to hand
    // itself a token it already stored is a round trip to nowhere — and
    // one more thing that can fail on a machine whose own overlay is
    // down.
    let me = crate::network::me(database)
        .await
        .ok()
        .flatten()
        .map(|me| me.id);
    let (host, wanted): (String, Vec<String>) = (
        hostname.to_string(),
        nodes
            .iter()
            .filter(|node| Some(*node) != me.as_ref())
            .cloned()
            .collect(),
    );
    let secret = wabot::prelude::password::generate(32);
    database
        .write(move |connection| {
            connection.execute("DELETE FROM edge_peer WHERE \"hostname\" = ?1", [&host])?;
            for node in &wanted {
                connection.execute(
                    "INSERT INTO edge_peer (\"hostname\", \"node_id\", \"updated_at\") \
                     VALUES (?1, ?2, ?3)",
                    (&host, node, now_ms()),
                )?;
            }
            connection.execute(
                "INSERT INTO edge_secret (\"hostname\", \"secret\", \"updated_at\") \
                 VALUES (?1, ?2, ?3) ON CONFLICT (\"hostname\") DO NOTHING",
                (&host, secret, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Store the secret a name's owner handed over.
///
/// Replaces whatever was there: this arrives on an errand from the node
/// that mints it, so what it says is the current answer by definition.
pub async fn accept_secret(
    database: &SqliteDatabase,
    hostname: &str,
    secret: &str,
) -> PlatformResult<()> {
    let (host, secret) = (hostname.to_string(), secret.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO edge_secret (\"hostname\", \"secret\", \"updated_at\") \
                 VALUES (?1, ?2, ?3) ON CONFLICT (\"hostname\") DO UPDATE SET \
                   \"secret\" = excluded.\"secret\", \
                   \"updated_at\" = excluded.\"updated_at\"",
                (&host, secret, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// The other nodes that answer for this name, as this node knows them.
pub async fn peers_for(database: &SqliteDatabase, hostname: &str) -> PlatformResult<Vec<String>> {
    let host = hostname.to_string();
    Ok(database
        .read(move |connection| {
            let mut statement = connection.prepare(
                "SELECT \"node_id\" FROM edge_peer WHERE \"hostname\" = ?1 \
                 ORDER BY \"node_id\"",
            )?;
            let rows = statement.query_map([host], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<String>, _>>()
        })
        .await?)
}

/// The secret this node holds for a name, if it holds one.
pub async fn secret_for(
    database: &SqliteDatabase,
    hostname: &str,
) -> PlatformResult<Option<String>> {
    let host = hostname.to_string();
    Ok(database
        .read(move |connection| {
            use wabot::sqlite::rusqlite::OptionalExtension;
            connection
                .query_row(
                    "SELECT \"secret\" FROM edge_secret WHERE \"hostname\" = ?1",
                    [host],
                    |row| row.get(0),
                )
                .optional()
        })
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::services;

    async fn service() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = crate::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        let service = crate::features::services::create_service_command::CreateService {
            project_id: &project.id,
            name: "web",
            image: "alpine:3.23",
            env: &[],
        }
        .run(&database)
        .await
        .expect("service");
        (database, service.id)
    }

    #[tokio::test]
    async fn a_name_can_be_served_by_several_nodes_at_once() {
        let (database, service) = service().await;
        set(
            &database,
            &service,
            "app.example.com",
            &["nd-one".into(), "nd-two".into()],
        )
        .await
        .expect("set");

        assert_eq!(
            nodes_for(&database, "app.example.com")
                .await
                .expect("query"),
            vec!["nd-one".to_string(), "nd-two".to_string()]
        );
    }

    /// A node dropped from the list still has to be told. It keeps
    /// answering for the name until an errand says otherwise, and
    /// nothing else in the system would notice — so the caller is
    /// handed exactly who to send that errand to.
    #[tokio::test]
    async fn dropping_a_node_says_which_one_to_tell() {
        let (database, service) = service().await;
        set(
            &database,
            &service,
            "app.example.com",
            &["nd-one".into(), "nd-two".into()],
        )
        .await
        .expect("set");

        let dropped = set(&database, &service, "app.example.com", &["nd-one".into()])
            .await
            .expect("set");

        assert_eq!(dropped, vec!["nd-two".to_string()]);
        assert_eq!(
            nodes_for(&database, "app.example.com")
                .await
                .expect("query"),
            vec!["nd-one".to_string()]
        );
    }

    /// Two names on one service are two independent choices — one may
    /// be served from everywhere and another from one node.
    #[tokio::test]
    async fn each_name_is_chosen_on_its_own() {
        let (database, service) = service().await;
        set(&database, &service, "one.example", &["nd-one".into()])
            .await
            .expect("set");
        set(&database, &service, "two.example", &["nd-two".into()])
            .await
            .expect("set");

        assert_eq!(
            of_service(&database, &service).await.expect("query"),
            vec![
                ("one.example".to_string(), "nd-one".to_string()),
                ("two.example".to_string(), "nd-two".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn edges_go_with_the_service() {
        let (database, service) = service().await;
        set(&database, &service, "app.example.com", &["nd-one".into()])
            .await
            .expect("set");

        services::delete(&database, &service).await.expect("delete");
        assert!(of_service(&database, &service)
            .await
            .expect("query")
            .is_empty());
    }
}
