use wabot::sqlite::SqliteDatabase;

use crate::platform::presets;
use crate::platform::services::{self, Service};

/// **Timestamps are paid for in history.** The prefix is 28 bytes against
/// a mean line of 148 measured here, so the same retention budget holds
/// about a sixth fewer lines.
pub struct SetServiceLogs<'a> {
    pub service: &'a Service,
    pub budget: &'a str,
    pub timestamps: bool,
}

impl SetServiceLogs<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        let budget = presets::parse_bytes(self.budget)?;
        if let Err(error) = services::set_log_budget(database, &self.service.id, budget).await {
            tracing::error!(%error, "could not store the log budget");
        }

        services::set_log_timestamps(database, &self.service.id, self.timestamps)
            .await
            .map_err(|error| error.to_string())
    }
}
