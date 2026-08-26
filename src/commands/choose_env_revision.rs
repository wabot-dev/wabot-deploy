use wabot::sqlite::SqliteDatabase;

use crate::platform::config_history;
use crate::platform::services::Service;

pub struct ChooseEnvRevision<'a> {
    pub service: &'a Service,
    pub revision_id: &'a str,
}

impl ChooseEnvRevision<'_> {
    /// Only this service's history: a revision id from another service
    /// would write somebody else's secrets into this one.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Vec<(String, String)>, String> {
        let Some(revision) = config_history::find(database, self.revision_id)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Err("no such revision".into());
        };
        if revision.service_id != self.service.id {
            return Err("that revision belongs to another service".into());
        }
        Ok(revision.env.into_iter().collect())
    }
}
