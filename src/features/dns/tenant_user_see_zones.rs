use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::values::capability::Capability;
use crate::features::dns::organisation_zones_response::OrganisationZonesResponse;
use crate::features::dns::zone_response::NameServer;
use crate::features::dns::zones_response::{ZoneOwner, ZoneRow};
use crate::infra::repositories::zones;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
) -> Result<OrganisationZonesResponse, String> {
    if !tenant_user::administers(account) {
        return Err("only an administrator of this organisation can do that".into());
    }
    let unreadable = |error: String| {
        tracing::warn!(%error, "could not read the zones");
        "the zones could not be read".to_string()
    };
    let tenant = account.tenant_id.clone().unwrap_or_default();

    let served: Vec<String> = crate::infra::dns::what_this_node_serves(database)
        .await
        .map_err(unreadable)?
        .into_iter()
        .map(|served| served.zone.id)
        .collect();
    let mut rows = Vec::new();
    for zone in zones::all(database)
        .await
        .map_err(|error| unreadable(error.to_string()))?
    {
        if zone.tenant_id.as_deref() != Some(tenant.as_str()) || !zone.is_ours() {
            continue;
        }
        let records = zones::records(database, &zone)
            .await
            .map(|records| records.len())
            .unwrap_or_default();
        rows.push(ZoneRow {
            owner: ZoneOwner::ThisNode,
            answered_here: served.contains(&zone.id),
            id: zone.id,
            name: zone.name.to_string(),
            records,
        });
    }

    let me = crate::infra::network::all(database)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .into_iter()
        .find(|node| node.is_self)
        .map(|node| node.id);
    let offered: Vec<String> = me
        .into_iter()
        .chain(
            crate::infra::network::candidates_for(database, Capability::Dns)
                .await
                .map_err(|error| unreadable(error.to_string()))?
                .into_iter()
                .map(|node| node.id),
        )
        .collect();
    let answering = crate::infra::dns::answering(database, &offered)
        .await
        .map_err(unreadable)?;
    let published = crate::infra::dns::published_names(database)
        .await
        .map_err(unreadable)?;
    let mut name_servers: Vec<NameServer> = Vec::new();
    for name in answering.iter().filter_map(|node| published.get(node)) {
        if name_servers.iter().any(|server| &server.name == name) {
            continue;
        }
        name_servers.push(NameServer {
            name: name.clone(),
            addresses: crate::infra::deploy::dns::lookup(name)
                .await
                .into_iter()
                .map(|address| address.to_string())
                .collect(),
        });
    }

    Ok(OrganisationZonesResponse {
        zones: rows,
        name_servers,
    })
}
