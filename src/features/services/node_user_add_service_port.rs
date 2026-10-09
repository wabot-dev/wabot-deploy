use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::port::Port;
use crate::domain::entities::service::Service;
use crate::features::services::add_service_port_request::AddServicePortRequest;
use crate::infra::config::Config;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    service: &Service,
    asked: &AddServicePortRequest,
) -> Result<Port, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to open".into());
    }

    crate::features::services::add_service_port_command::AddServicePort { service, asked }
        .run(database, config)
        .await
}
