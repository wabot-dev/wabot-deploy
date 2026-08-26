use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::delete_service::DeleteService;
use crate::deploy::Deployer;
use crate::platform::projects::Project;
use crate::platform::services::Service;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    service: &Service,
    confirm: &str,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
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
