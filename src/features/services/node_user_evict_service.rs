use wabot::sqlite::SqliteDatabase;

use crate::authz::node_user;
use crate::deploy::Deployer;
use crate::entities::account::Account;
use crate::entities::project::Project;
use crate::entities::service::Service;
use crate::features::services::evict_service_command::EvictService;

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
