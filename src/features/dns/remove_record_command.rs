use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::dns_zone::DnsZone;
use crate::domain::errors::PlatformError;
use crate::features::dns::add_record_command::{told, SENT_FROM_ELSEWHERE};
use crate::infra::repositories::zones;

pub struct RemoveRecord<'a> {
    pub zone: &'a DnsZone,
    pub record_id: &'a str,
}

impl RemoveRecord<'_> {
    pub async fn run(
        &self,
        database: &SqliteDatabase,
        container: &Container,
    ) -> Result<(), String> {
        if !self.zone.is_ours() {
            return Err(SENT_FROM_ELSEWHERE.into());
        }
        zones::remove_record(database, self.zone, self.record_id)
            .await
            .map_err(|error| match error {
                PlatformError::Refused(reason) => reason,
                other => {
                    tracing::warn!(error = %other, "could not remove a record");
                    "that could not be saved".to_string()
                }
            })?;
        told(database, container, self.zone).await;
        Ok(())
    }
}
