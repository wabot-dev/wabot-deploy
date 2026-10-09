use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::features::services::set_service_logs_command::SetServiceLogs;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    budget: &str,
    timestamps: bool,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
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
