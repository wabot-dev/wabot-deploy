use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::set_service_edges::SetServiceEdges;
use crate::config::Config;
use crate::platform::services::Service;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    service: &Service,
    hostname: &str,
    chosen: &[String],
) -> Result<Vec<String>, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to route".into());
    }

    SetServiceEdges {
        service,
        hostname,
        chosen,
    }
    .run(database, config)
    .await
}
