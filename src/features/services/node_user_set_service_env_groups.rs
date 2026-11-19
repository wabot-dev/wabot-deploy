use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::features::services::env_applied_response::EnvAppliedResponse;
use crate::features::services::set_service_env_groups_command::SetServiceEnvGroups;
use crate::features::services::set_service_env_groups_request::SetServiceEnvGroupsRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    request: &SetServiceEnvGroupsRequest,
) -> Result<EnvAppliedResponse, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    SetServiceEnvGroups {
        service,
        group_ids: &request.group_ids,
    }
    .run(database)
    .await
}
