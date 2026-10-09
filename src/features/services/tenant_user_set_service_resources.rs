use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::features::services::set_service_resources_request::SetServiceResourcesRequest;
use crate::infra::deploy::Deployer;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    service: &Service,
    asked: &SetServiceResourcesRequest,
) -> Result<Service, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    crate::features::services::set_service_resources_command::SetServiceResources {
        project,
        service,
        asked,
    }
    .run(database, deployer)
    .await
}
