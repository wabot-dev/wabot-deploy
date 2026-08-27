use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::add_service_port::{AddServicePort, Asked};
use crate::config::Config;
use crate::entities::port::Port;
use crate::entities::service::Service;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    service: &Service,
    asked: Asked<'_>,
) -> Result<Port, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to open".into());
    }

    AddServicePort { service, asked }
        .run(database, config)
        .await
}
