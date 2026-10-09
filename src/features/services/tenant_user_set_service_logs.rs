use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::entities::service::Service;
use crate::features::services::set_service_logs_command::SetServiceLogs;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    budget: &str,
    timestamps: bool,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    SetServiceLogs {
        service,
        budget,
        timestamps,
    }
    .run(database)
    .await
}
