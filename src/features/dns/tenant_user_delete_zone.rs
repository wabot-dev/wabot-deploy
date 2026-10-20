use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::dns::delete_zone_command::DeleteZone;
use crate::features::dns::tenant_user_see_zone::theirs;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    zone_name: &str,
) -> Result<(), String> {
    let zone = theirs(database, account, zone_name).await?;
    DeleteZone { zone: &zone }.run(database, container).await
}
