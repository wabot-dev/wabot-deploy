use std::collections::BTreeMap;

use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::capability::Capability;
use crate::features::dns::zone_response::{NameServer, RecordRow, ZoneResponse};
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

    let mut name_servers = Vec::new();
    for server in &servers {
        if let Some(name) = published.get(server) {
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
        id: zone.id,
        name: zone.name.to_string(),
        serial: zone.serial,
    })
}
