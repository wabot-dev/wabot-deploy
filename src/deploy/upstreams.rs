//! Where an edge sends the traffic for one name.
//!
//! Pure: replicas in, addresses out. **The weight is the repetition** —
//! a node running two copies appears twice and gets twice the requests,
//! and nothing computes a ratio.

pub fn upstreams(
    placements: &[crate::entities::replica::Replica],
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
    use crate::entities::replica::Replica;

    fn node(id: &str, overlay: &str, is_self: bool) -> crate::network::Node {
        crate::network::Node {
            update_note: None,
            version: None,
            wanted_version: None,
            usage: None,
            draining_at: None,
            capacity: None,
            id: id.into(),
            name: id.into(),
            kind: crate::network::Kind::Public,
            endpoint: Some("198.51.100.1:51820".into()),
            public_key: None,
            overlay_ip: Some(overlay.into()),
            is_self,
            last_seen_at: None,
            allows: Vec::new(),
            ca_pem: None,
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
            memory_bytes: None,
            disk_bytes: None,
            cpu_millicores: None,
            following: None,
            wal_held: None,
            placed_by: None,
            placed_note: None,
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
}
