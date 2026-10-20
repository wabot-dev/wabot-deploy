use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::errors::PlatformError;
use crate::domain::values::zone_name::ZoneName;
use crate::features::dns::create_zone_request::CreateZoneRequest;
use crate::features::dns::zone_created_response::ZoneCreatedResponse;
use crate::infra::repositories::zones;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    asked: &CreateZoneRequest,
) -> Result<ZoneCreatedResponse, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let name = ZoneName::parse(&asked.name)?;
    let me = crate::infra::network::all(database)
        .await
        .ok()
        .and_then(|nodes| nodes.into_iter().find(|node| node.is_self))
        .ok_or_else(|| "this node does not know its own id yet — restart it once".to_string())?;

    let zone = zones::create(
        database,
        zones::NewZone {
            name: &name,
            tenant_id: None,
            servers: &[me.id],
        },
    )
    .await
    .map_err(|error| match error {
        PlatformError::Refused(reason) => reason,
        other => {
            tracing::warn!(error = %other, "could not create a zone");
            "that could not be saved".to_string()
        }
    })?;

    crate::infra::dns::reload(container, database).await;
    crate::infra::dns::dispatch::tell_servers(database, &zone, &[]).await;
    Ok(ZoneCreatedResponse {
        name: zone.name.to_string(),
    })
}
