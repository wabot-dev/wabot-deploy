use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::infra::config::Config;

pub async fn run(
    database: &Arc<SqliteDatabase>,
    config: &Config,
    account: &Account,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if crate::infra::backup::plan::in_progress() {
        return Err("a backup is already running on this node".into());
    }

    crate::infra::backup::plan::start_in_background(config.clone(), database.clone());
    Ok(())
}
