use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::features::buckets::create_store_command::CreateStore;
use crate::features::buckets::create_store_request::CreateStoreRequest;
use crate::infra::repositories::access;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &CreateStoreRequest,
) -> Result<(Project, Service), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
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
    let service = CreateStore {
        project: &project,
        asked,
    }
    .run(database)
    .await?;
    Ok((project, service))
}
