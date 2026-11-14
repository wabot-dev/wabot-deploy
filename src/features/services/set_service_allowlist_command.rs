use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::Service;
use crate::domain::values::allowlist::{Allowlist, Door};
use crate::infra::deploy::Deployer;
use crate::infra::repositories::allowlists;

/// Who may connect to every name this service answers on.
///
/// **No deployment.** The edge decides per request from a table it reloads,
/// so the list is in force when this returns — here at once, and on the
/// other nodes serving its names as soon as they collect the instruction.
pub struct SetServiceAllowlist<'a> {
    pub service: &'a Service,
    pub allowlist: &'a Allowlist,
}

impl SetServiceAllowlist<'_> {
    pub async fn run(&self, database: &SqliteDatabase, deployer: &Deployer) -> Result<(), String> {
        if !self.service.is_ours() {
            return Err("this service is administered from the node that sent it".into());
        }
        allowlists::set(
            database,
            &Door::Service(self.service.id.clone()),
            self.allowlist,
        )
        .await
        .map_err(|error| error.to_string())?;
        deployer.tell_edges(self.service).await;
        deployer.sync_routes().await;
        Ok(())
    }
}
