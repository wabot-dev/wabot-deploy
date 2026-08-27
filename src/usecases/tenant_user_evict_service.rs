use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::evict_service::EvictService;
use crate::deploy::Deployer;
use crate::entities::project::Project;
use crate::entities::service::Service;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    service: &Service,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to evict".into());
    }

    EvictService { project, service }
        .run(database, deployer)
        .await
}
