use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::errors::PlatformError;
use crate::domain::values::capability::Capability;
use crate::domain::values::zone_name::ZoneName;
use crate::features::dns::zone_created_response::ZoneCreatedResponse;
use crate::infra::repositories::zones;

/// `with_every_name_server` is the difference between the two actors: an
/// operator starts with this node and chooses the rest on the zone's page,
/// an organisation chooses nothing and is answered by every node that
/// offers to.
pub struct CreateZone<'a> {
    pub name: &'a ZoneName,
    pub tenant_id: Option<&'a str>,
    pub with_every_name_server: bool,
}

impl CreateZone<'_> {
    pub async fn run(
        &self,
        database: &SqliteDatabase,
        container: &Container,
    ) -> Result<ZoneCreatedResponse, String> {
        let me = crate::infra::network::all(database)
            .await
            .ok()
            .and_then(|nodes| nodes.into_iter().find(|node| node.is_self))
            .ok_or_else(|| {
                "this node does not know its own id yet — restart it once".to_string()
            })?;
        let mut servers = vec![me.id.clone()];
        if self.with_every_name_server {
            let others = crate::infra::network::candidates_for(database, Capability::Dns)
                .await
                .unwrap_or_default();
            servers.extend(
                others
                    .into_iter()
                    .map(|node| node.id)
                    .filter(|id| *id != me.id),
            );
        }

        let zone = zones::create(
            database,
            zones::NewZone {
                name: self.name,
                tenant_id: self.tenant_id,
                servers: &servers,
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
}
