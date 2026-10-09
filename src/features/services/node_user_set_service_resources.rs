use wabot::sqlite::SqliteDatabase;

use crate::authz::node_user;
use crate::deploy::Deployer;
use crate::entities::account::Account;
use crate::entities::project::Project;
use crate::entities::service::Service;
use crate::features::services::set_service_resources_request::SetServiceResourcesRequest;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    service: &Service,
    asked: &SetServiceResourcesRequest,
) -> Result<Service, String> {
    if !node_user::belongs_here(account) {
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
