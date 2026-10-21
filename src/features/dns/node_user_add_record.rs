use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::dns::add_record_command::AddRecord;
use crate::features::dns::add_record_request::AddRecordRequest;
use crate::features::dns::node_user_see_zone::theirs;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    asked: &AddRecordRequest,
) -> Result<(), String> {
    let (_, allowed, zone) = theirs(database, account, &asked.project, &asked.zone).await?;
    if !allowed.may_deploy() {
        return Err("you may look at this project, not change it".into());
    }
    AddRecord { zone: &zone, asked }
        .run(database, container)
        .await
}
