use wabot::sqlite::SqliteDatabase;

use crate::platform::ports;
use crate::platform::services::Service;

pub struct DeleteServicePort<'a> {
    pub service: &'a Service,
    pub port_id: &'a str,
}

impl DeleteServicePort<'_> {
    /// Only this service's ports: an id from another page must not delete
    /// somebody else's.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        let owned = ports::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .any(|port| port.id == self.port_id);
        if !owned {
            return Err("no such port here".into());
        }

        ports::delete(database, self.port_id)
            .await
            .map_err(|error| error.to_string())
    }
}
