use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::dns::check_delegation_command::CheckDelegation;
use crate::features::dns::node_user_see_zone::theirs;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &str,
    zone_name: &str,
) -> Result<(), String> {
    let (_, allowed, zone) = theirs(database, account, project, zone_name).await?;
    if !allowed.may_deploy() {
        return Err("you may look at this project, not change it".into());
    }
    CheckDelegation { zone: &zone }
        .run(database)
        .await
        .map_err(|error| {
            tracing::warn!(%error, zone = %zone.name, "could not check the delegation");
            "that could not be saved".to_string()
        })
}
