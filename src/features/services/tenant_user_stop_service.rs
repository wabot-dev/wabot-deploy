use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::features::services::stop_service_command::StopService;
use crate::infra::deploy::Deployer;

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
