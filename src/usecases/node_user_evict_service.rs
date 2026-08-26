use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::evict_service::EvictService;
use crate::deploy::Deployer;
use crate::platform::projects::Project;
use crate::platform::services::Service;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    service: &Service,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to evict".into());
    }

    EvictService { project, service }
        .run(database, deployer)
        .await
}
