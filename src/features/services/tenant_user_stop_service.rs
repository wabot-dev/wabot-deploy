use crate::authz::tenant_user;
use crate::deploy::Deployer;
use crate::entities::account::Account;
use crate::entities::project::Project;
use crate::entities::service::Service;
use crate::features::services::stop_service_command::StopService;

pub async fn run(
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    service: &Service,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to stop".into());
    }

    StopService { project, service }.run(deployer).await
}
