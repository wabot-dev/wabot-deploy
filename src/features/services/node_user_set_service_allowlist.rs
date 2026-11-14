use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::features::services::set_service_allowlist_command::SetServiceAllowlist;
use crate::features::services::set_service_allowlist_request::SetServiceAllowlistRequest;
use crate::infra::deploy::Deployer;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    service: &Service,
    request: &SetServiceAllowlistRequest,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    SetServiceAllowlist {
        service,
        allowlist: &request.allowlist,
    }
    .run(database, deployer)
    .await
}
