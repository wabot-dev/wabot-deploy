use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::features::account::add_passkey_command::AddPasskey;
use crate::features::account::add_passkey_request::AddPasskeyRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &AddPasskeyRequest,
) -> Result<crate::features::account::passkey_added_response::PasskeyAddedResponse, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that account is not this console's".into());
    }
    AddPasskey { account, asked }.run(database).await
}
