use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::dns::add_record_command::AddRecord;
use crate::features::dns::add_record_request::AddRecordRequest;
use crate::features::dns::tenant_user_see_zone::theirs;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    asked: &AddRecordRequest,
) -> Result<(), String> {
    let zone = theirs(database, account, &asked.zone).await?;
    AddRecord { zone: &zone, asked }
        .run(database, container)
        .await
}
