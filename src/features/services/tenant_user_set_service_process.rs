use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::features::services::set_service_process_command::SetServiceProcess;
use crate::features::services::set_service_process_request::SetServiceProcessRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    request: &SetServiceProcessRequest,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    SetServiceProcess {
        service,
        entrypoint: request.entrypoint.as_ref(),
        command: request.command.as_ref(),
    }
    .run(database)
    .await
}
