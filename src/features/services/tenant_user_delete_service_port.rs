use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::entities::service::Service;
use crate::features::services::delete_service_port_command::DeleteServicePort;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    port_id: &str,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    DeleteServicePort { service, port_id }.run(database).await
}
