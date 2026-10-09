use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::features::databases::restore_database_command::RestoreDatabase;
use crate::infra::backup::Restored;
use crate::infra::config::Config;

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
