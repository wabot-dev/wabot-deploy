use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::domain::errors::PlatformError;
use crate::domain::values::capability::Capability;
use crate::domain::values::zone_name::ZoneName;
use crate::features::dns::zone_created_response::ZoneCreatedResponse;
use crate::infra::repositories::zones;

pub struct CreateZone<'a> {
    pub name: &'a ZoneName,
    pub project: &'a Project,
}

impl CreateZone<'_> {
    pub async fn run(
        &self,
        database: &SqliteDatabase,
        container: &Container,
    ) -> Result<ZoneCreatedResponse, String> {
        let servers = first_servers(database, self.project).await?;
        let zone = zones::create(
            database,
            zones::NewZone {
                name: self.name,
                project: self.project,
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

/// Who answers for a zone the moment it is made. The operator's own
/// projects start with this node and the operator chooses the rest on the
/// zone's page; an organisation's start with every node that offers to,
/// because an organisation chooses none.
pub async fn first_servers(
    database: &SqliteDatabase,
    project: &Project,
) -> Result<Vec<String>, String> {
    let me = crate::infra::network::all(database)
        .await
        .ok()
        .and_then(|nodes| nodes.into_iter().find(|node| node.is_self))
        .ok_or_else(|| "this node does not know its own id yet — restart it once".to_string())?;
    let mut servers = vec![me.id.clone()];
    if !project.is_the_nodes_own() {
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
    Ok(servers)
}
