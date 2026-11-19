use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::infra::repositories::{allowlist_sets, services};

pub struct DeleteAllowlistSet<'a> {
    pub project: &'a Project,
    pub name: &'a str,
}

impl DeleteAllowlistSet<'_> {
    /// Refused while a service uses it, rather than taking it off them:
    /// what that would close is a decision about each of those services.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if !self.project.is_ours() {
            return Err("this project is administered from the node that sent it".into());
        }
        let Some(set) = allowlist_sets::find(database, &self.project.id, self.name)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(());
        };
        let mut names = Vec::new();
        for service_id in allowlist_sets::services_using(database, &set.id)
            .await
            .map_err(|error| error.to_string())?
        {
            if let Some(service) = services::find(database, &service_id)
                .await
                .map_err(|error| error.to_string())?
            {
                names.push(service.name);
            }
        }
        if !names.is_empty() {
            return Err(format!(
                "{} is used by {} — take it off them first",
                set.name,
                names.join(", ")
            ));
        }
        allowlist_sets::delete(database, &set.id)
            .await
            .map_err(|error| error.to_string())
    }
}
