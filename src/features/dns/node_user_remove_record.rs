use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::features::dns::remove_record_command::RemoveRecord;
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
    RemoveRecord {
        zone: &zone,
        record_id,
    }
    .run(database, container)
    .await
}
