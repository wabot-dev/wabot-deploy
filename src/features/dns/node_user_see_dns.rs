use std::collections::BTreeMap;

use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::capability::Capability;
use crate::features::dns::dns_response::{DnsResponse, HeldZone, OwnName};
use crate::infra::dns::Zones;
use crate::infra::repositories::zones;

pub async fn run(
    database: &SqliteDatabase,
    table: Option<&Zones>,
    account: &Account,
) -> Result<DnsResponse, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let unreadable = |error: String| {
        tracing::warn!(%error, "could not read the zones");
        "the zones could not be read".to_string()
    };

    let served: Vec<String> = crate::infra::dns::what_this_node_serves(database)
        .await
        .map_err(unreadable)?
        .into_iter()
        .map(|served| served.zone.id)
        .collect();
    let nodes: BTreeMap<String, String> = crate::infra::network::all(database)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .into_iter()
        .map(|node| (node.id, node.name))
        .collect();

    let mut held = Vec::new();
    for zone in zones::all(database)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .into_iter()
        .filter(|zone| zone.project_id.is_none())
    {
        let records = zones::records(database, &zone)
            .await
            .map(|records| records.len())
            .unwrap_or_default();
        held.push(HeldZone {
            sent_by: zone.authority_id.as_ref().map(|authority| {
                nodes
                    .get(authority)
                    .cloned()
                    .unwrap_or_else(|| authority.clone())
            }),
            answered_here: served.contains(&zone.id),
            name: zone.name.to_string(),
            records,
        });
    }

    Ok(DnsResponse {
        provides_dns: crate::infra::network::capability::provides(database, Capability::Dns).await,
        addressed: crate::infra::node::settings::domain(database)
            .await
            .is_some(),
        answering_any: !served.is_empty(),
        listening: table
            .map(|table| table.listening().to_vec())
            .unwrap_or_default(),
        declared: crate::infra::dns::names::declared(database).await,
        answers_as: crate::infra::dns::names::where_each_points(database)
            .await
            .into_iter()
            .map(|pointing| OwnName {
                addresses: pointing
                    .addresses
                    .iter()
                    .map(|address| address.to_string())
                    .collect(),
                here: pointing.here,
                name: pointing.name,
            })
            .collect(),
        held,
    })
}
