use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::set_service_resources::{Asked, SetServiceResources};
use crate::deploy::Deployer;
use crate::entities::project::Project;
use crate::entities::service::Service;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    service: &Service,
    asked: Asked<'_>,
) -> Result<Service, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    SetServiceResources {
        project,
        service,
        asked,
    }
    .run(database, deployer)
    .await
}
