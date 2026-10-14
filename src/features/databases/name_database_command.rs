use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::features::databases::database_named_response::DatabaseNamedResponse;
use crate::infra::repositories::ports;

pub struct NameDatabase<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    pub typed: &'a str,
}

impl NameDatabase<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<DatabaseNamedResponse, String> {
        let typed = crate::domain::values::hostname::Hostname::parse(self.typed)
            .as_str()
            .to_string();
        if typed.is_empty() {
            return Err("a database is reached by name, so it has to have one".into());
        }

        let was = crate::infra::deploy::certificate_names(database, self.project, self.service)
            .await
            .into_iter()
            .next();

        let Some(port) = ports::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .next()
        else {
            return Err("this database has no port row".into());
        };

        ports::set_hostname(database, &port.id, Some(&typed))
            .await
            .map_err(|error| error.to_string())?;

        Ok(DatabaseNamedResponse {
            was: was.filter(|was| was != &typed),
            now: typed,
        })
    }
}
