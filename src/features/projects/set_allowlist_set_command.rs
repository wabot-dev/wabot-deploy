use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::domain::values::allowlist::Allowlist;
use crate::infra::deploy::Deployer;
use crate::infra::repositories::{allowlist_sets, services};

pub struct SetAllowlistSet<'a> {
    pub project: &'a Project,
    pub name: &'a str,
    pub list: &'a Allowlist,
}

impl SetAllowlistSet<'_> {
    /// No deployment, as for a service's own list: the edge and the kernel
    /// reload theirs, and the nodes serving a service's names are told.
    pub async fn run(&self, database: &SqliteDatabase, deployer: &Deployer) -> Result<(), String> {
        if !self.project.is_ours() {
            return Err("this project is administered from the node that sent it".into());
        }
        let Some(set) = allowlist_sets::find(database, &self.project.id, self.name)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Err("no such list in this project".into());
        };
        allowlist_sets::set_list(database, &set.id, self.list)
            .await
            .map_err(|error| error.to_string())?;
        for service_id in allowlist_sets::services_using(database, &set.id)
            .await
            .map_err(|error| error.to_string())?
        {
            if let Some(service) = services::find(database, &service_id)
                .await
                .map_err(|error| error.to_string())?
            {
                deployer.tell_edges(&service).await;
            }
        }
        deployer.sync_routes().await;
        Ok(())
    }
}
