use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::dns_zone::DnsZone;
use crate::infra::repositories::zones;

/// A zone sent by another node is deleted here too, and that is eviction:
/// the operator of a machine may always put down what somebody else asked
/// it to carry. Every node that answered for one of this node's own is told
/// to stop.
pub struct DeleteZone<'a> {
    pub zone: &'a DnsZone,
}

impl DeleteZone<'_> {
    pub async fn run(
        &self,
        database: &SqliteDatabase,
        container: &Container,
    ) -> Result<(), String> {
        let servers = zones::servers(database, &self.zone.id)
            .await
            .unwrap_or_default();
        zones::delete(database, self.zone).await.map_err(|error| {
            tracing::warn!(%error, "could not delete a zone");
            "that could not be saved".to_string()
        })?;
        crate::infra::dns::reload(container, database).await;
        crate::infra::dns::dispatch::tell_servers(database, self.zone, &servers).await;
        Ok(())
    }
}
