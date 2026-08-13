//! Which public nodes answer for a service's hostname.
//!
//! Chosen by the node that owns the service, like where its replicas
//! run. See migration `0023`.

use wabot::sqlite::SqliteDatabase;

use super::{now_ms, PlatformResult};

/// Set the nodes that serve `hostname`, and say which stopped.
///
/// The ones removed come back because they still have to be told: a
/// node that was serving a name and is no longer chosen keeps answering
/// for it until an errand says otherwise, and nothing else in the
/// system will notice.
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

/// Where every running copy of a service answers, from the overlay.
///
/// **One entry per replica**, so a node holding two appears twice in the
/// list and a round-robin sends it twice the requests. That is the whole
/// of the load balancing: the edge picks by turn, and weight is how many
/// times a node appears.
///
/// Each entry is the *node's* overlay address and the port bound to it,
/// never the container's own — a container address is on a bridge that
/// is not unique across nodes, so it would name a different container on
/// the machine reading it.
///
/// A copy with no port yet is left out rather than guessed at: it has
/// reported none, so nothing can reach it, and an invented address in
/// this list is a request that hangs instead of one that fails over.
pub fn upstreams(
    placements: &[super::replicas::Replica],
    nodes: &[crate::network::Node],
) -> Vec<String> {
    let mut upstreams = Vec::new();

    for replica in placements.iter().filter(|replica| !replica.evicted()) {
        let Some(port) = replica.overlay_port else {
            continue;
        };
        let overlay = match &replica.node_id {
            Some(node_id) => nodes.iter().find(|node| &node.id == node_id),
            None => nodes.iter().find(|node| node.is_self),
        }
        .and_then(|node| node.overlay_ip.clone());

        if let Some(overlay) = overlay {
            upstreams.push(format!("{overlay}:{port}"));
        }
    }
    upstreams
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{projects, replicas::Replica, services};

    async fn service() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "demo").await.expect("project");
        let service = services::create(&database, &project.id, "web", "alpine:3.23", &[])
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

    fn node(id: &str, overlay: &str, is_self: bool) -> crate::network::Node {
        crate::network::Node {
            id: id.into(),
            name: id.into(),
            kind: crate::network::Kind::Public,
            endpoint: Some("198.51.100.1:51820".into()),
            public_key: None,
            overlay_ip: Some(overlay.into()),
            is_self,
            last_seen_at: None,
            allows: Vec::new(),
        }
    }

    fn replica(node_id: Option<&str>, slot: u32, port: Option<u16>) -> Replica {
        Replica {
            id: format!("rep-{slot}"),
            service_id: "svc".into(),
            node_id: node_id.map(str::to_string),
            slot,
            address: None,
            overlay_port: port,
            last_error: None,
            evicted_at: None,
            reserved_host: None,
        }
    }

    /// The load balancing, and the whole of it: a node running two
    /// copies appears twice, so a round-robin over this list sends it
    /// twice the requests of a node running one.
    #[test]
    fn a_node_with_two_replicas_appears_twice() {
        let nodes = [
            node("nd-one", "10.42.0.1", true),
            node("nd-two", "10.42.0.2", false),
        ];
        let placements = [
            replica(None, 1, Some(30000)),
            replica(None, 2, Some(30001)),
            replica(Some("nd-two"), 3, Some(30000)),
        ];

        let upstreams = upstreams(&placements, &nodes);
        assert_eq!(
            upstreams,
            vec![
                "10.42.0.1:30000".to_string(),
                "10.42.0.1:30001".to_string(),
                "10.42.0.2:30000".to_string(),
            ]
        );
        assert_eq!(
            upstreams
                .iter()
                .filter(|up| up.starts_with("10.42.0.1"))
                .count(),
            2,
            "the node with two copies has to be twice as likely"
        );
    }

    /// A replica with no overlay port has reported nowhere to reach it.
    /// Guessing one would put an address in the list that answers
    /// nothing — a request that hangs rather than one that fails over.
    #[test]
    fn a_replica_with_no_port_is_left_out() {
        let nodes = [node("nd-one", "10.42.0.1", true)];
        let placements = [replica(None, 1, None), replica(None, 2, Some(30001))];

        assert_eq!(
            upstreams(&placements, &nodes),
            vec!["10.42.0.1:30001".to_string()]
        );
    }

    /// A copy the node running it threw out is gone, and an edge still
    /// pointing at it is sending a share of the traffic nowhere.
    #[test]
    fn an_evicted_replica_stops_receiving_traffic() {
        let nodes = [node("nd-two", "10.42.0.2", false)];
        let mut evicted = replica(Some("nd-two"), 1, Some(30000));
        evicted.evicted_at = Some(1);

        assert!(upstreams(&[evicted], &nodes).is_empty());
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
