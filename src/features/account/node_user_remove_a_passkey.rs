use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::features::account::remove_passkey_command::RemovePasskey;
use crate::features::account::remove_passkey_request::RemovePasskeyRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &RemovePasskeyRequest,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that account is not this console's".into());
    }
    RemovePasskey { account, asked }.run(database).await
}
