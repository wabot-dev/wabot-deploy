use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::features::dns::check_delegation_command::CheckDelegation;
use crate::infra::repositories::zones;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    zone_name: &str,
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
    CheckDelegation { zone: &zone }
        .run(database)
        .await
        .map_err(|error| {
            tracing::warn!(%error, zone = %zone.name, "could not check the delegation");
            "that could not be saved".to_string()
        })
}
