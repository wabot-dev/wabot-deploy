//! Telling the other nodes that answer for a zone what it says now.
//!
//! Beside `Deployer::tell_edges` and for the same reasons: the whole zone
//! is rebuilt from the rows every time and sent only when it changed, so
//! a change made from the console and a boot that re-sends everything are
//! one code path, and saying the same thing twice says nothing.

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::dns_zone::DnsZone;
use crate::domain::values::capability::Capability;
use crate::infra::network::errand::{self, Kind};
use crate::infra::repositories::zones;

/// Send the zone to every node chosen for it except this one, and a
/// withdrawal to each of `removed`, the nodes that were chosen and are not
/// now.
pub async fn tell_servers(database: &SqliteDatabase, zone: &DnsZone, removed: &[String]) {
    if !zone.is_ours() {
        return;
    }
    let (Ok(servers), Ok(records), Ok(candidates), Ok(nodes)) = (
        zones::servers(database, &zone.id).await,
        zones::records(database, zone).await,
        crate::infra::network::candidates_for(database, Capability::Dns).await,
        crate::infra::network::all(database).await,
    ) else {
        tracing::warn!(zone = %zone.name, "could not work out who answers for it");
        return;
    };
    let Ok(name_servers) = super::name_servers(database, &servers).await else {
        tracing::warn!(zone = %zone.name, "could not work out who answers for it");
        return;
    };
    let me = nodes
        .iter()
        .find(|node| node.is_self)
        .map(|node| node.id.clone());
    let allowed: Vec<String> = candidates.into_iter().map(|node| node.id).collect();

    let current = errand::Zone {
        name: zone.name.to_string(),
        serial: zone.serial,
        default_ttl: zone.default_ttl,
        records: records
            .iter()
            .map(|record| errand::ZoneRecord {
                name: record.name.to_string(),
                kind: record.data.kind().name().to_string(),
                value: record.data.value(),
                priority: record.data.priority(),
                ttl: record.ttl,
            })
            .collect(),
        name_servers,
        withdrawn: false,
    };
    let subject = format!("zone:{}", zone.id);

    for node in &servers {
        if Some(node) == me.as_ref() {
            continue;
        }
        if !allowed.contains(node) {
            tracing::debug!(%node, zone = %zone.name, "skipped: it has not agreed to answer DNS for this node");
            continue;
        }
        send(database, node, &subject, &current).await;
    }

    let withdrawal = errand::Zone {
        records: Vec::new(),
        withdrawn: true,
        ..current
    };
    for node in removed {
        if Some(node) == me.as_ref() || servers.contains(node) {
            continue;
        }
        send(database, node, &subject, &withdrawal).await;
    }
}

/// Every zone this node administers, at boot: whatever changed while it
/// was down reaches the others, and an unchanged zone costs one read.
pub async fn tell_all(database: &SqliteDatabase) {
    match zones::all(database).await {
        Ok(zones) => {
            for zone in zones.iter().filter(|zone| zone.is_ours()) {
                tell_servers(database, zone, &[]).await;
            }
        }
        Err(error) => tracing::warn!(%error, "could not read the zones to send them"),
    }
}

async fn send(database: &SqliteDatabase, node: &str, subject: &str, zone: &errand::Zone) {
    let payload = match serde_json::to_value(zone) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(%error, "could not build a zone instruction");
            return;
        }
    };
    match errand::queue_if_changed(database, node, Kind::Zone, subject, &payload).await {
        Ok(Some(_)) => {
            tracing::info!(%node, zone = %zone.name, withdrawn = zone.withdrawn, "told a node about a zone")
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, %node, "could not queue a zone instruction"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::record_data::{RecordData, RecordKind};
    use crate::domain::values::record_name::RecordName;
    use crate::domain::values::zone_name::ZoneName;
    use crate::infra::config::Config;
    use crate::infra::network::{Kind as NodeKind, Node};

    fn peer(id: &str, name: &str, overlay: &str, allows: Vec<Capability>) -> Node {
        Node {
            update_note: None,
            version: None,
            wanted_version: None,
            usage: None,
            draining_at: None,
            capacity: None,
            id: id.into(),
            name: name.into(),
            kind: NodeKind::Public,
            endpoint: Some(format!("{name}:443")),
            public_key: None,
            overlay_ip: Some(overlay.into()),
            is_self: false,
            last_seen_at: None,
            allows,
            ca_pem: None,
        }
    }

    async fn network() -> (SqliteDatabase, String) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::node::settings::set_domain(&database, Some("node-0.example.net"))
            .await
            .expect("domain");
        let me = crate::infra::network::ensure_self(&database, &Config::default())
            .await
            .expect("self")
            .id;
        for node in [
            peer(
                "nd-yes",
                "node-1.example.net",
                "10.42.0.8",
                vec![Capability::Dns],
            ),
            peer("nd-no", "node-2.example.net", "10.42.0.9", vec![]),
        ] {
            crate::infra::network::save(&database, &node)
                .await
                .expect("peer");
        }
        (database, me)
    }

    async fn zone(database: &SqliteDatabase, servers: &[String]) -> DnsZone {
        let name = ZoneName::parse("teresaasprilla.com").expect("zone");
        let zone = zones::create(
            database,
            zones::NewZone {
                name: &name,
                tenant_id: None,
                servers,
            },
        )
        .await
        .expect("zone");
        zones::add_record(
            database,
            &zone,
            RecordName::apex(),
            RecordData::parse(RecordKind::A, "209.50.48.176", None).expect("data"),
            300,
        )
        .await
        .expect("record");
        zones::find_by_name(database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("there")
    }

    async fn zone_errands(database: &SqliteDatabase) -> Vec<errand::Record> {
        errand::all(database)
            .await
            .expect("errands")
            .into_iter()
            .filter(|record| record.kind == Kind::Zone)
            .collect()
    }

    /// A node that never agreed is neither sent the zone nor named as one
    /// of its name servers: resolvers sent to it would hear nothing.
    #[tokio::test]
    async fn a_zone_goes_only_to_chosen_nodes_that_agreed() {
        let (database, me) = network().await;
        let zone = zone(&database, &[me, "nd-yes".into(), "nd-no".into()]).await;

        tell_servers(&database, &zone, &[]).await;
        let sent = zone_errands(&database).await;
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0].node_id, "nd-yes");
        let payload: errand::Zone =
            serde_json::from_value(sent[0].payload.clone()).expect("a zone");
        assert_eq!(
            payload.name_servers,
            vec!["node-0.example.net", "node-1.example.net"]
        );
        assert_eq!(payload.records.len(), 1);
        assert!(!payload.withdrawn);

        tell_servers(&database, &zone, &[]).await;
        assert_eq!(
            zone_errands(&database).await.len(),
            1,
            "saying the same thing twice says nothing"
        );
    }

    /// The zone a peer is sent lists every server under the names it
    /// answers as, so the NS records it serves agree with the ones served
    /// here — and a registrar that asks for two gets two from one node.
    #[tokio::test]
    async fn a_zone_names_each_server_as_it_answers() {
        let (database, me) = network().await;
        let ours = crate::domain::values::name_server::NameServerName::all(&[
            "ns0.example.net".into(),
            "ns1.example.net".into(),
        ])
        .expect("names");
        crate::infra::dns::names::declare(&database, &ours)
            .await
            .expect("declared");
        crate::infra::dns::names::remember(&database, "nd-yes", &["ns2.example.net".into()])
            .await
            .expect("said");
        let zone = zone(&database, &[me, "nd-yes".into()]).await;

        tell_servers(&database, &zone, &[]).await;
        let sent = zone_errands(&database).await;
        let payload: errand::Zone =
            serde_json::from_value(sent[0].payload.clone()).expect("a zone");
        assert_eq!(
            payload.name_servers,
            vec!["ns0.example.net", "ns1.example.net", "ns2.example.net"]
        );

        let served = crate::infra::dns::what_this_node_serves(&database)
            .await
            .expect("served");
        assert_eq!(served[0].name_servers, payload.name_servers);
    }

    #[tokio::test]
    async fn a_node_taken_off_a_zone_is_told_to_stop() {
        let (database, me) = network().await;
        let zone = zone(&database, &[me.clone(), "nd-yes".into()]).await;
        tell_servers(&database, &zone, &[]).await;

        zones::set_servers(&database, &zone, &[me])
            .await
            .expect("servers");
        let zone = zones::find_by_name(&database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("there");
        tell_servers(&database, &zone, &["nd-yes".to_string()]).await;

        let last = zone_errands(&database)
            .await
            .into_iter()
            .next()
            .expect("an errand");
        assert_eq!(last.node_id, "nd-yes");
        let payload: errand::Zone = serde_json::from_value(last.payload).expect("a zone");
        assert!(payload.withdrawn, "the node that was dropped was not told");
    }
}
