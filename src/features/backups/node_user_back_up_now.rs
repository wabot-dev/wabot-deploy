use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::authz::node_user;
use crate::config::Config;
use crate::entities::account::Account;

pub async fn run(
    database: &Arc<SqliteDatabase>,
    config: &Config,
    account: &Account,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if crate::backup::plan::in_progress() {
        return Err("a backup is already running on this node".into());
    }

    crate::backup::plan::start_in_background(config.clone(), database.clone());
    Ok(())
}
