use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::config::Config;
use crate::dtos::requests::add_service_port::AddServicePort;
use crate::entities::port::Port;
use crate::entities::service::Service;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    service: &Service,
    asked: &AddServicePort,
) -> Result<Port, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to open".into());
    }

    crate::commands::add_service_port::AddServicePort { service, asked }
        .run(database, config)
        .await
}
