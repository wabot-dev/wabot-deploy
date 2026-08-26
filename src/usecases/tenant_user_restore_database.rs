use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::backup::Restored;
use crate::commands::restore_database::RestoreDatabase;
use crate::config::Config;
use crate::platform::services::Service;

pub async fn run(
    database: &Arc<SqliteDatabase>,
    config: &Config,
    account: &Account,
    service: &Service,
    at: &str,
    into: &str,
) -> Result<Restored, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to restore".into());
    }

    RestoreDatabase { service, at, into }
        .run(database, config)
        .await
}
