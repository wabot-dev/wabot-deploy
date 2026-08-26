use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::publish_database::PublishDatabase;
use crate::platform::services::Service;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    published: bool,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to publish".into());
    }

    PublishDatabase { service, published }.run(database).await
}
