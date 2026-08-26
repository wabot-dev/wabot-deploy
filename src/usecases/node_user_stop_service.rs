use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::stop_service::StopService;
use crate::deploy::Deployer;
use crate::platform::projects::Project;
use crate::platform::services::Service;

pub async fn run(
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    service: &Service,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to stop".into());
    }

    StopService { project, service }.run(deployer).await
}
