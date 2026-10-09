use crate::accounts::Account;
use crate::authz::node_user;
use crate::features::services::set_service_placement_command::{On, SetServicePlacement};

pub async fn run(
    on: On<'_>,
    account: &Account,
    command: SetServicePlacement<'_>,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to place".into());
    }

    command.run(on).await
}
