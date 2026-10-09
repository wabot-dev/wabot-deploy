use wabot::sqlite::SqliteDatabase;

use crate::authz::tenant_user;
use crate::config::Config;
use crate::entities::account::Account;
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
    if !tenant_user::belongs_here(account) {
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
