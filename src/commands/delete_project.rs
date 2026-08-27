use wabot::sqlite::SqliteDatabase;

use crate::deploy::Deployer;
use crate::platform::projects::{self, Project};
use crate::repositories::services;

pub struct DeleteProject<'a> {
    pub project: &'a Project,
    /// The project's own slug, typed. Checked here rather than trusted
    /// from the form: `required` is a courtesy the browser extends, and
    /// a request can be constructed by hand.
    pub confirm: &'a str,
}

impl DeleteProject<'_> {
    pub async fn run(&self, database: &SqliteDatabase, deployer: &Deployer) -> Result<(), String> {
        if self.confirm.trim() != self.project.slug {
            return Err("type the project's name to confirm the deletion".into());
        }

        // **Every service is taken down first, one at a time.** This used
        // to be the row deletion alone, and `ON DELETE CASCADE` took the
        // service rows with it — which is not the same as taking the
        // *services*. Nothing stopped a container and nothing removed a
        // volume, and the container id is derived from the rows, so
        // losing the rows loses the handle for ever.
        let all = services::all(database, Some(&self.project.id))
            .await
            .unwrap_or_default();
        for service in &all {
            deployer.tear_down(self.project, service).await;
            deployer.discard_storage(self.project, service).await;
        }

        projects::delete(database, &self.project.id)
            .await
            .map_err(|error| error.to_string())
    }
}
