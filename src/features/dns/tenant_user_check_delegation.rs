use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::dns::check_delegation_command::CheckDelegation;
use crate::features::dns::tenant_user_see_zone::theirs;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    zone_name: &str,
) -> Result<(), String> {
    let zone = theirs(database, account, zone_name).await?;
    CheckDelegation { zone: &zone }
        .run(database)
        .await
        .map_err(|error| {
            tracing::warn!(%error, zone = %zone.name, "could not check the delegation");
            "that could not be saved".to_string()
        })
}
