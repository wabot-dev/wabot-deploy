use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::config::Config;
use crate::entities::port::Port;
use crate::entities::service::Service;
use crate::features::services::add_service_port_request::AddServicePortRequest;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    service: &Service,
    asked: &AddServicePortRequest,
) -> Result<Port, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to open".into());
    }

    crate::features::services::add_service_port_command::AddServicePort { service, asked }
        .run(database, config)
        .await
}
