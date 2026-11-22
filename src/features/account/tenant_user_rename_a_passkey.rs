use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::features::account::rename_passkey_command::RenamePasskey;
use crate::features::account::rename_passkey_request::RenamePasskeyRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &RenamePasskeyRequest,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that account is not this console's".into());
    }
    RenamePasskey { account, asked }.run(database).await
}
