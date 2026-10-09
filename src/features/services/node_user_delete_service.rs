use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::deploy::Deployer;
use crate::entities::project::Project;
use crate::entities::service::Service;
use crate::features::services::delete_service_command::DeleteService;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    service: &Service,
    confirm: &str,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to delete".into());
    }

    DeleteService {
        project,
        service,
        confirm,
    }
    .run(database, deployer)
    .await
}
