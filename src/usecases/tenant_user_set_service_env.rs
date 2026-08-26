use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::set_service_env::{Applied, SetServiceEnv};
use crate::platform::services::Service;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    env: Vec<(String, String)>,
    reason: &str,
) -> Result<Applied, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    SetServiceEnv {
        service,
        env,
        by: Some(&account.id),
        reason,
    }
    .run(database)
    .await
}
