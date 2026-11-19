use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::infra::repositories::{env_groups, services};

pub struct DeleteEnvGroup<'a> {
    pub project: &'a Project,
    pub name: &'a str,
}

impl DeleteEnvGroup<'_> {
    /// Refused while a service joins it, rather than taking it off them:
    /// a service losing a variable it reads is a decision about that
    /// service, and nobody made it.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if !self.project.is_ours() {
            return Err("this project is administered from the node that sent it".into());
        }
        let Some(group) = env_groups::find(database, &self.project.id, self.name)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(());
        };
        let mut names = Vec::new();
        for service_id in env_groups::services_using(database, &group.id)
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
                "{} is joined by {} — take it off them first",
                group.name,
                names.join(", ")
            ));
        }
        env_groups::delete(database, &group.id)
            .await
            .map_err(|error| error.to_string())
    }
}
