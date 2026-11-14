use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::domain::values::allowlist::{Allowlist, Door};
use crate::infra::deploy::Deployer;
use crate::infra::repositories::allowlists;

/// Who may push to and pull from the project's own registry name.
///
/// Nodes of the network are let in whatever it says, so a copy placed
/// elsewhere can still pull its image: the edge admits the address a node
/// last called from.
pub struct SetRegistryAllowlist<'a> {
    pub project: &'a Project,
    pub allowlist: &'a Allowlist,
}

impl SetRegistryAllowlist<'_> {
    pub async fn run(&self, database: &SqliteDatabase, deployer: &Deployer) -> Result<(), String> {
        if !self.project.is_ours() {
            return Err("this project is administered from the node that sent it".into());
        }
        allowlists::set(
            database,
            &Door::ProjectRegistry(self.project.id.clone()),
            self.allowlist,
        )
        .await
        .map_err(|error| error.to_string())?;
        deployer.sync_routes().await;
        Ok(())
    }
}
