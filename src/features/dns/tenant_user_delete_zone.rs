use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::dns::delete_zone_command::DeleteZone;
use crate::features::dns::tenant_user_see_zone::theirs;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    project: &str,
    zone_name: &str,
) -> Result<(), String> {
    let (_, allowed, zone) = theirs(database, account, project, zone_name).await?;
    if !allowed.may_deploy() {
        return Err("you may look at this project, not change it".into());
    }
    DeleteZone { zone: &zone }.run(database, container).await
}
