use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::features::dns::create_zone_command::first_servers;
use crate::features::dns::zone_response::NameServer;
use crate::features::dns::zones_response::{ZoneRow, ZonesResponse};
use crate::infra::repositories::zones;

pub struct DescribeZones<'a> {
    pub project: &'a Project,
    pub editable: bool,
}

impl DescribeZones<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<ZonesResponse, String> {
        let unreadable = |error: String| {
            tracing::warn!(%error, project = %self.project.slug, "could not read the zones");
            "the zones could not be read".to_string()
        };
        let served: Vec<String> = crate::infra::dns::what_this_node_serves(database)
            .await
            .map_err(unreadable)?
            .into_iter()
            .map(|served| served.zone.id)
            .collect();

        let mut rows = Vec::new();
        for zone in zones::of_project(database, &self.project.id)
            .await
            .map_err(|error| unreadable(error.to_string()))?
        {
            let records = zones::records(database, &zone)
                .await
                .map(|records| records.len())
                .unwrap_or_default();
            rows.push(ZoneRow {
                answered_here: served.contains(&zone.id),
                name: zone.name.to_string(),
                records,
            });
        }

        let offered = first_servers(database, self.project).await?;
        let mut name_servers = Vec::new();
        for name in crate::infra::dns::name_servers(database, &offered)
            .await
            .map_err(unreadable)?
        {
            name_servers.push(NameServer {
                addresses: crate::infra::deploy::dns::lookup(&name)
                    .await
                    .into_iter()
                    .map(|address| address.to_string())
                    .collect(),
                name,
            });
        }

        Ok(ZonesResponse {
            zones: rows,
            name_servers,
            editable: self.editable,
        })
    }
}
