use std::collections::BTreeMap;

use wabot::sqlite::SqliteDatabase;

use crate::entities::service::Service;
use crate::platform::releases;
use crate::repositories::config_history;
use crate::repositories::services;

pub struct SetServiceEnv<'a> {
    pub service: &'a Service,
    pub env: Vec<(String, String)>,
    pub by: Option<&'a str>,
    /// What the history row says happened: an edit, or a revert.
    pub reason: &'a str,
}

pub struct Applied {
    pub service: Service,
    /// The release that is deployed, so the redeployment keeps the image
    /// it is on rather than moving to the newest one.
    pub release_id: Option<String>,
}

impl SetServiceEnv<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Applied, String> {
        if self.service.kind.is_managed() {
            return Err("the node writes this for a managed database".into());
        }

        let map: BTreeMap<String, String> = self.env.iter().cloned().collect();
        services::set_env(database, &self.service.id, &map)
            .await
            .map_err(|error| error.to_string())?;
        config_history::record(database, &self.service.id, &map, self.by, self.reason)
            .await
            .map_err(|error| error.to_string())?;

        let release_id = releases::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|release| release.deployed_at.is_some())
            .map(|release| release.id);

        Ok(Applied {
            service: Service {
                env: map,
                ..self.service.clone()
            },
            release_id,
        })
    }
}
