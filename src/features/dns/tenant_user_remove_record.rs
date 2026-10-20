use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::dns::remove_record_command::RemoveRecord;
use crate::features::dns::tenant_user_see_zone::theirs;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    zone_name: &str,
    record_id: &str,
) -> Result<(), String> {
    let zone = theirs(database, account, zone_name).await?;
    RemoveRecord {
        zone: &zone,
        record_id,
    }
    .run(database, container)
    .await
}
