use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::entities::service::Service;
use crate::features::services::env_applied_response::EnvAppliedResponse;
use crate::features::services::set_service_env_command::SetServiceEnv;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    env: Vec<(String, String)>,
    reason: &str,
) -> Result<EnvAppliedResponse, String> {
    if !tenant_user::belongs_here(account) {
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
    .map(|applied| EnvAppliedResponse {
        service_id: applied.service.id,
        release_id: applied.release_id,
    })
}
