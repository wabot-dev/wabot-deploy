use std::collections::BTreeMap;

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::env_group::{clash, EnvGroup};
use crate::domain::entities::project::Project;
use crate::domain::entities::service::DesiredState;
use crate::features::projects::env_group_saved_response::{EnvGroupSavedResponse, Redeploy};
use crate::infra::repositories::{env_groups, releases, services};

pub struct SetEnvGroup<'a> {
    pub project: &'a Project,
    pub name: &'a str,
    pub env: &'a BTreeMap<String, String>,
}

impl SetEnvGroup<'_> {
    /// A stopped service is not redeployed: a deployment is what starts
    /// one, and it reads the group when somebody does.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<EnvGroupSavedResponse, String> {
        if !self.project.is_ours() {
            return Err("this project is administered from the node that sent it".into());
        }
        let Some(group) = env_groups::find(database, &self.project.id, self.name)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Err("no such group in this project".into());
        };
        let saved = EnvGroup {
            env: self.env.clone(),
            ..group
        };

        let mut redeploy = Vec::new();
        for service_id in env_groups::services_using(database, &saved.id)
            .await
            .map_err(|error| error.to_string())?
        {
            let Some(service) = services::find(database, &service_id)
                .await
                .map_err(|error| error.to_string())?
            else {
                continue;
            };
            let others: Vec<EnvGroup> = env_groups::of_service(database, &service.id)
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|other| other.id != saved.id)
                .collect();
            if let Some(clash) = clash(others.iter().chain(std::iter::once(&saved))) {
                return Err(format!("{}, as {} does", clash.sentence(), service.name));
            }
            if service.desired_state == DesiredState::Running
                && service.is_ours()
                && !service.kind.is_managed()
            {
                let release_id = releases::of_service(database, &service.id)
                    .await
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .find(|release| release.deployed_at.is_some())
                    .map(|release| release.id);
                redeploy.push(Redeploy {
                    service_id: service.id.clone(),
                    release_id,
                });
            }
        }

        env_groups::set_env(database, &saved.id, &saved.env)
            .await
            .map_err(|error| error.to_string())?;
        Ok(EnvGroupSavedResponse { redeploy })
    }
}
