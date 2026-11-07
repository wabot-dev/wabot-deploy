use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::features::services::set_service_placement_command::{On, SetServicePlacement};
use crate::infra::repositories::access;

pub async fn run(
    on: On<'_>,
    account: &Account,
    command: SetServicePlacement<'_>,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to place".into());
    }

    let theirs = access::find_project(on.database, account, &command.project.slug)
        .await
        .map_err(|error| error.to_string())?
        .is_some_and(|(project, allowed)| {
            project.id == command.project.id
                && command.service.project_id == project.id
                && allowed.may_deploy()
        });
    if !theirs {
        return Err("no such service".into());
    }

    command.run(on).await
}
