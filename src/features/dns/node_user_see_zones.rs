use std::collections::BTreeMap;

use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::capability::Capability;
use crate::features::dns::zones_response::{ZoneOwner, ZoneRow, ZonesResponse};
use crate::infra::dns::Zones;
use crate::infra::repositories::{tenants, zones};

pub async fn run(
    database: &SqliteDatabase,
    table: Option<&Zones>,
    account: &Account,
) -> Result<ZonesResponse, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let unreadable = |error: String| {
        tracing::warn!(%error, "could not read the zones");
        "the zones could not be read".to_string()
    };

    let all = zones::all(database)
        .await
        .map_err(|error| unreadable(error.to_string()))?;
    let served: Vec<String> = crate::infra::dns::what_this_node_serves(database)
        .await
        .map_err(unreadable)?
        .into_iter()
        .map(|served| served.zone.id)
        .collect();
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

    let mut rows = Vec::new();
    for zone in all {
        let records = zones::records(database, &zone)
            .await
            .map(|records| records.len())
            .unwrap_or_default();
        rows.push(ZoneRow {
            owner: ZoneOwner::of(&zone, &organisations, &nodes),
            answered_here: served.contains(&zone.id),
            id: zone.id,
            name: zone.name.to_string(),
            records,
        });
    }

    Ok(ZonesResponse {
        zones: rows,
        provides_dns: crate::infra::network::capability::provides(database, Capability::Dns).await,
        addressed: crate::infra::node::settings::domain(database)
            .await
            .is_some(),
        listening: table
            .map(|table| table.listening().to_vec())
            .unwrap_or_default(),
    })
}
