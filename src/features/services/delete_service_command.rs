use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::infra::deploy::Deployer;
use crate::infra::repositories::services;

pub struct DeleteService<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    /// The service's own slug, typed. Checked here rather than trusted
    /// from the form: `required` is a courtesy the browser extends.
    pub confirm: &'a str,
}

impl DeleteService<'_> {
    /// **In order: the container, then the storage, then the rows.** The
    /// container id is derived from the rows, so deleting the rows first
    /// loses the handle and nothing could ever clean them up again.
    pub async fn run(&self, database: &SqliteDatabase, deployer: &Deployer) -> Result<(), String> {
        if self.confirm.trim() != self.service.slug {
            return Err("type the service's name to confirm the deletion".into());
        }

        deployer.tear_down(self.project, self.service).await;
        deployer.discard_storage(self.project, self.service).await;
        services::delete(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())
    }
}
