use wabot::sqlite::SqliteDatabase;

use crate::authz::node_user;
use crate::entities::account::Account;
use crate::entities::service::Service;
use crate::features::databases::publish_database_command::PublishDatabase;

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
