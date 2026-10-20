use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::errors::PlatformError;
use crate::features::dns::node_user_add_record::SENT_FROM_ELSEWHERE;
use crate::infra::repositories::zones;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    zone_name: &str,
    record_id: &str,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let zone = zones::find_by_name(database, zone_name)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "could not read a zone");
            "the zones could not be read".to_string()
        })?
        .ok_or_else(|| zones::NO_SUCH_ZONE.to_string())?;
    if !zone.is_ours() {
        return Err(SENT_FROM_ELSEWHERE.into());
    }

    zones::remove_record(database, &zone, record_id)
        .await
        .map_err(|error| match error {
            PlatformError::Refused(reason) => reason,
            other => {
                tracing::warn!(error = %other, "could not remove a record");
                "that could not be saved".to_string()
            }
        })?;

    crate::infra::dns::reload(container, database).await;
    Ok(())
}
