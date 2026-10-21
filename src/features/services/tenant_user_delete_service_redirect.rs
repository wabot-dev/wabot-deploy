use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::features::services::delete_service_redirect_command::DeleteServiceRedirect;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    hostname: &str,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }
    DeleteServiceRedirect { service, hostname }
        .run(database)
        .await
}
