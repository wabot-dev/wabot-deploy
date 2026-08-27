use wabot::sqlite::SqliteDatabase;

use crate::entities::service::Service;
use crate::platform::ports;

/// The primary only. A pool answering from outside would need a port per
/// replica and something choosing between them, which is a load balancer
/// and has its own justification rather than being a detail of this.
pub struct PublishDatabase<'a> {
    pub service: &'a Service,
    pub published: bool,
}

impl PublishDatabase<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        let Some(port) = ports::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .next()
        else {
            return Err("this database has no port row".into());
        };

        ports::set_published(database, &port.id, self.published)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
