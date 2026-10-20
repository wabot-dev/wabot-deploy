use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::dns_record::BAD_TTL;
use crate::domain::entities::dns_zone::DEFAULT_TTL;
use crate::domain::errors::PlatformError;
use crate::domain::values::record_data::{RecordData, RecordKind};
use crate::domain::values::record_name::RecordName;
use crate::features::dns::add_record_request::AddRecordRequest;
use crate::infra::repositories::zones;

pub const SENT_FROM_ELSEWHERE: &str =
    "this zone was sent by another node, and it is changed there — this node only answers for it";

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    asked: &AddRecordRequest,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let zone = zones::find_by_name(database, &asked.zone)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "could not read a zone");
            "the zones could not be read".to_string()
        })?
        .ok_or_else(|| zones::NO_SUCH_ZONE.to_string())?;
    if !zone.is_ours() {
        return Err(SENT_FROM_ELSEWHERE.into());
    }

    let name = RecordName::parse(&asked.name, &zone.name)?;
    let kind = RecordKind::parse(&asked.kind)?;
    let priority = Some(asked.priority.as_str()).filter(|priority| !priority.trim().is_empty());
    let data = RecordData::parse(kind, &asked.value, priority)?;
    let ttl = match asked.ttl.trim() {
        "" => DEFAULT_TTL,
        typed => typed.parse().map_err(|_| BAD_TTL.to_string())?,
    };

    zones::add_record(database, &zone, name, data, ttl)
        .await
        .map_err(|error| match error {
            PlatformError::Refused(reason) => reason,
            other => {
                tracing::warn!(error = %other, "could not add a record");
                "that could not be saved".to_string()
            }
        })?;

    crate::infra::dns::reload(container, database).await;
    if let Ok(Some(zone)) = zones::find_by_name(database, zone.name.as_str()).await {
        crate::infra::dns::dispatch::tell_servers(database, &zone, &[]).await;
    }
    Ok(())
}
