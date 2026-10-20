use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::dns_record::BAD_TTL;
use crate::domain::entities::dns_zone::{DnsZone, DEFAULT_TTL};
use crate::domain::errors::PlatformError;
use crate::domain::values::record_data::{RecordData, RecordKind};
use crate::domain::values::record_name::RecordName;
use crate::features::dns::add_record_request::AddRecordRequest;
use crate::infra::repositories::zones;

pub const SENT_FROM_ELSEWHERE: &str =
    "this zone was sent by another node, and it is changed there — this node only answers for it";

pub struct AddRecord<'a> {
    pub zone: &'a DnsZone,
    pub asked: &'a AddRecordRequest,
}

impl AddRecord<'_> {
    pub async fn run(
        &self,
        database: &SqliteDatabase,
        container: &Container,
    ) -> Result<(), String> {
        let zone = self.zone;
        if !zone.is_ours() {
            return Err(SENT_FROM_ELSEWHERE.into());
        }
        let name = RecordName::parse(&self.asked.name, &zone.name)?;
        let kind = RecordKind::parse(&self.asked.kind)?;
        let priority =
            Some(self.asked.priority.as_str()).filter(|priority| !priority.trim().is_empty());
        let data = RecordData::parse(kind, &self.asked.value, priority)?;
        let ttl = match self.asked.ttl.trim() {
            "" => DEFAULT_TTL,
            typed => typed.parse().map_err(|_| BAD_TTL.to_string())?,
        };

        zones::add_record(database, zone, name, data, ttl)
            .await
            .map_err(|error| match error {
                PlatformError::Refused(reason) => reason,
                other => {
                    tracing::warn!(error = %other, "could not add a record");
                    "that could not be saved".to_string()
                }
            })?;
        told(database, container, zone).await;
        Ok(())
    }
}

/// After a change: the listener reads the rows again, and the other nodes
/// that answer for the zone are sent the new one — which has a new serial,
/// so it is read back rather than taken from the zone the caller held.
pub async fn told(database: &SqliteDatabase, container: &Container, zone: &DnsZone) {
    crate::infra::dns::reload(container, database).await;
    if let Ok(Some(zone)) = zones::find_by_name(database, zone.name.as_str()).await {
        crate::infra::dns::dispatch::tell_servers(database, &zone, &[]).await;
    }
}
