use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::Service;
use crate::infra::deploy::Deployer;
use crate::infra::repositories::allowlist_sets;

pub struct SetServiceAllowlistSets<'a> {
    pub service: &'a Service,
    pub set_ids: &'a [String],
}

impl SetServiceAllowlistSets<'_> {
    pub async fn run(&self, database: &SqliteDatabase, deployer: &Deployer) -> Result<(), String> {
        if !self.service.is_ours() {
            return Err("this service is administered from the node that sent it".into());
        }
        let project = allowlist_sets::of_project(database, &self.service.project_id)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(stranger) = self
            .set_ids
            .iter()
            .find(|id| !project.iter().any(|set| &&set.id == id))
        {
            return Err(format!("{stranger} is no list of this project"));
        }
        allowlist_sets::set_for_service(database, &self.service.id, self.set_ids)
            .await
            .map_err(|error| error.to_string())?;
        deployer.tell_edges(self.service).await;
        deployer.sync_routes().await;
        Ok(())
    }
}
