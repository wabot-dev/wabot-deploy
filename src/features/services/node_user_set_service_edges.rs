use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::config::Config;
use crate::entities::service::Service;
use crate::features::services::set_service_edges_command::SetServiceEdges;

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
