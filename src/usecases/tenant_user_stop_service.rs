use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::stop_service::StopService;
use crate::deploy::Deployer;
use crate::entities::service::Service;
use crate::platform::projects::Project;

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
