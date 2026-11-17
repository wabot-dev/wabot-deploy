use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::features::buckets::restore_store_command::RestoreStore;
use crate::features::buckets::restore_store_request::RestoreStoreRequest;
use crate::features::buckets::store_restored_response::StoreRestoredResponse;
use crate::infra::config::Config;
use crate::infra::repositories::{access, services};

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    asked: &RestoreStoreRequest,
) -> Result<StoreRestoredResponse, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to restore".into());
    }
    let Some((project, allowed)) = access::find_project(database, account, &asked.project)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such project".into());
    };
    if !allowed.may_deploy() {
        return Err("you cannot deploy in this project".into());
    }
    let service = services::in_project(database, &project.id, &asked.store)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("no such store")?;
    RestoreStore {
        project: &project,
        service: &service,
        asked,
    }
    .run(database, config)
    .await
}
