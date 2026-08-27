use wabot::sqlite::SqliteDatabase;

use crate::entities::service::Service;
use crate::platform::databases;
use crate::policies::presets;

/// An unbounded Postgres on a node with one core is the node, so the
/// form's empty answer is the form's default answer rather than "no
/// ceiling".
pub const DEFAULT_PRESET: u64 = 256 * 1024 * 1024;

/// What a form asks for, before anybody has decided whose project it
/// is. Shared so a controller serving both consoles builds it once and
/// hands it to whichever actor's use case the door belongs to.
#[derive(Clone, Copy)]
pub struct Asked<'a> {
    pub project: &'a str,
    pub name: &'a str,
    pub version: &'a str,
    pub memory: &'a str,
}

pub struct CreateDatabase<'a> {
    pub project_id: &'a str,
    pub name: &'a str,
    pub version: &'a str,
    pub memory: &'a str,
}

impl CreateDatabase<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Service, String> {
        let memory = presets::parse(self.memory)?.unwrap_or(DEFAULT_PRESET);
        let (service, _) =
            databases::create(database, self.project_id, self.name, self.version, memory)
                .await
                .map_err(|error| error.to_string())?;
        Ok(service)
    }
}
