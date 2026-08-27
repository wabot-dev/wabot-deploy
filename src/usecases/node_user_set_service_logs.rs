use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::set_service_logs::SetServiceLogs;
use crate::entities::service::Service;

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
