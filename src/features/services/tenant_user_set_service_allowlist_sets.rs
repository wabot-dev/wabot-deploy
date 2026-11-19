use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::features::services::set_service_allowlist_sets_command::SetServiceAllowlistSets;
use crate::features::services::set_service_allowlist_sets_request::SetServiceAllowlistSetsRequest;
use crate::infra::deploy::Deployer;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    service: &Service,
    request: &SetServiceAllowlistSetsRequest,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    SetServiceAllowlistSets {
        service,
        set_ids: &request.set_ids,
    }
    .run(database, deployer)
    .await
}
