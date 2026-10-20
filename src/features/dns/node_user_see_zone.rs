use std::collections::BTreeMap;

use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::capability::Capability;
use crate::features::dns::zone_response::{
    Delivery, NameServer, RecordRow, ServerChoice, ZoneResponse,
};
use crate::features::dns::zones_response::ZoneOwner;
use crate::infra::repositories::{tenants, zones};

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    zone_name: &str,
) -> Result<ZoneResponse, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let unreadable = |error: String| {
        tracing::warn!(%error, zone = %zone_name, "could not read a zone");
        "the zones could not be read".to_string()
    };

    let zone = zones::find_by_name(database, zone_name)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .ok_or_else(|| zones::NO_SUCH_ZONE.to_string())?;
    let records = zones::records(database, &zone)
        .await
        .map_err(|error| unreadable(error.to_string()))?;
    let servers = zones::servers(database, &zone.id)
        .await
        .map_err(|error| unreadable(error.to_string()))?;
    let published = crate::infra::dns::published_names(database)
        .await
        .map_err(unreadable)?;
    let answered_here = crate::infra::dns::what_this_node_serves(database)
        .await
        .map_err(unreadable)?
        .iter()
        .any(|served| served.zone.id == zone.id);
    let organisations: BTreeMap<String, String> = tenants::all(database)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .into_iter()
        .map(|tenant| (tenant.id, tenant.name))
        .collect();
    let nodes: BTreeMap<String, String> = crate::infra::network::all(database)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .into_iter()
        .map(|node| (node.id, node.name))
        .collect();

    let servers_offered = match zone.is_ours() {
        true => choices(database, &zone.name.to_string(), &servers, &published).await,
        false => Vec::new(),
    };

    let mut name_servers = Vec::new();
    let named: Vec<String> = match zone.is_ours() {
        true => servers
            .iter()
            .filter_map(|server| published.get(server).cloned())
            .collect(),
        false => zone.received_name_servers.clone(),
    };
    for name in &named {
        let addresses = crate::infra::deploy::dns::lookup(name)
            .await
            .into_iter()
            .map(|address| address.to_string())
            .collect();
        name_servers.push(NameServer {
            name: name.clone(),
            addresses,
        });
    }

    Ok(ZoneResponse {
        owner: ZoneOwner::of(&zone, &organisations, &nodes),
        editable: zone.is_ours(),
        answered_here,
        provides_dns: crate::infra::network::capability::provides(database, Capability::Dns).await,
        records: records
            .iter()
            .map(|record| RecordRow {
                id: record.id.clone(),
                name: record.name.to_string(),
                fqdn: record.name.fqdn(&zone.name),
                kind: record.data.kind().name(),
                value: record.data.value(),
                priority: record.data.priority(),
                ttl: record.ttl,
            })
            .collect(),
        name_servers,
        servers: servers_offered,
        id: zone.id,
        name: zone.name.to_string(),
        serial: zone.serial,
    })
}

/// Every node that could answer for the zone or was chosen to, and what
/// each was last sent: this one needs no errand, the others are told.
async fn choices(
    database: &SqliteDatabase,
    zone: &str,
    chosen: &[String],
    published: &BTreeMap<String, String>,
) -> Vec<ServerChoice> {
    let nodes = crate::infra::network::all(database)
        .await
        .unwrap_or_default();
    let agreed: Vec<String> = crate::infra::network::candidates_for(database, Capability::Dns)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|node| node.id)
        .collect();
    let errands = crate::infra::network::errand::all(database)
        .await
        .unwrap_or_default();
    nodes
        .into_iter()
        .filter(|node| node.is_self || agreed.contains(&node.id) || chosen.contains(&node.id))
        .map(|node| {
            let last = (!node.is_self)
                .then(|| {
                    errands.iter().find(|errand| {
                        errand.kind == crate::infra::network::errand::Kind::Zone
                            && errand.node_id == node.id
                            && errand.payload.get("name").and_then(|name| name.as_str())
                                == Some(zone)
                    })
                })
                .flatten()
                .map(|errand| {
                    let withdrawn = errand
                        .payload
                        .get("withdrawn")
                        .and_then(|withdrawn| withdrawn.as_bool())
                        .unwrap_or(false);
                    match (&errand.error, errand.done_at, withdrawn) {
                        (Some(reason), _, _) => Delivery::Refused(reason.clone()),
                        (None, Some(_), true) => Delivery::Withdrawn,
                        (None, Some(_), false) => Delivery::Answering,
                        (None, None, _) => Delivery::Waiting,
                    }
                });
            ServerChoice {
                published: published.get(&node.id).cloned(),
                this_node: node.is_self,
                chosen: chosen.contains(&node.id),
                agreed: node.is_self || agreed.contains(&node.id),
                node_name: node.name,
                node_id: node.id,
                last,
            }
        })
        .collect()
}
