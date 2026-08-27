use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::backup::Restored;
use crate::commands::restore_database::RestoreDatabase;
use crate::config::Config;
use crate::entities::service::Service;

pub async fn run(
    database: &Arc<SqliteDatabase>,
    config: &Config,
    account: &Account,
    service: &Service,
    at: &str,
    into: &str,
) -> Result<Restored, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to restore".into());
    }

    RestoreDatabase { service, at, into }
        .run(database, config)
        .await
}
