use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::set_service_env::SetServiceEnv;
use crate::dtos::responses::env_applied::EnvApplied;
use crate::entities::service::Service;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    env: Vec<(String, String)>,
    reason: &str,
) -> Result<EnvApplied, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    SetServiceEnv {
        service,
        env,
        by: Some(&account.id),
        reason,
    }
    .run(database)
    .await
    // The entity stops here. What the caller does with this is queue a
    // deployment, and it only ever needed the row named.
    .map(|applied| EnvApplied {
        service_id: applied.service.id,
        release_id: applied.release_id,
    })
}
