use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::delete_service_port::DeleteServicePort;
use crate::platform::services::Service;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    port_id: &str,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    DeleteServicePort { service, port_id }.run(database).await
}
