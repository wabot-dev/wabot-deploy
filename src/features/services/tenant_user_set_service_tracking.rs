use wabot::sqlite::SqliteDatabase;

use crate::authz::tenant_user;
use crate::entities::account::Account;
use crate::entities::service::Service;
use crate::features::services::set_service_tracking_command::SetServiceTracking;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    tag: &str,
    auto_deploy: bool,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    SetServiceTracking {
        service,
        tag,
        auto_deploy,
    }
    .run(database)
    .await
}
