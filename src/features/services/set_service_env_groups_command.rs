use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::env_group::{clash, EnvGroup};
use crate::domain::entities::service::Service;
use crate::features::services::env_applied_response::EnvAppliedResponse;
use crate::infra::repositories::{env_groups, releases};

pub struct SetServiceEnvGroups<'a> {
    pub service: &'a Service,
    pub group_ids: &'a [String],
}

impl SetServiceEnvGroups<'_> {
    /// Read off this service's own project: a group id from another one
    /// would hand this service somebody else's secrets.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<EnvAppliedResponse, String> {
        if self.service.kind.is_managed() {
            return Err("the node writes this for a managed database".into());
        }
        if !self.service.is_ours() {
            return Err("this service is administered from the node that sent it".into());
        }
        let project = env_groups::of_project(database, &self.service.project_id)
            .await
            .map_err(|error| error.to_string())?;
        let mut chosen: Vec<&EnvGroup> = Vec::with_capacity(self.group_ids.len());
        for id in self.group_ids {
            match project.iter().find(|group| &group.id == id) {
                Some(group) => chosen.push(group),
                None => return Err("no such group in this project".into()),
            }
        }
        if let Some(clash) = clash(chosen.iter().copied()) {
            return Err(clash.sentence());
        }
        let ids: Vec<String> = chosen.iter().map(|group| group.id.clone()).collect();
        env_groups::set_for_service(database, &self.service.id, &ids)
            .await
            .map_err(|error| error.to_string())?;

        let release_id = releases::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|release| release.deployed_at.is_some())
            .map(|release| release.id);
        Ok(EnvAppliedResponse {
            service_id: self.service.id.clone(),
            release_id,
        })
    }
}
