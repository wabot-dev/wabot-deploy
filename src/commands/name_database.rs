use wabot::sqlite::SqliteDatabase;

use crate::config::Config;
use crate::platform::ports;
use crate::platform::projects::Project;
use crate::platform::services::Service;

pub struct Named {
    /// What it answered to before, when the rename replaced a name.
    pub was: Option<String>,
    pub now: String,
}

pub struct NameDatabase<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    pub typed: &'a str,
}

impl NameDatabase<'_> {
    pub async fn run(&self, database: &SqliteDatabase, config: &Config) -> Result<Named, String> {
        let typed = ports::normalize_hostname(self.typed);
        if typed.is_empty() {
            return Err("a database is reached by name, so it has to have one".into());
        }

        let was = crate::deploy::certificate_names(database, config, self.project, self.service)
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

        Ok(Named {
            was: was.filter(|was| was != &typed),
            now: typed,
        })
    }
}
