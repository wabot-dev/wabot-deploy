use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::env_group_saved_response::EnvGroupSavedResponse;
use crate::features::projects::set_env_group_command::SetEnvGroup;
use crate::features::projects::set_env_group_request::SetEnvGroupRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    name: &str,
    request: &SetEnvGroupRequest,
) -> Result<EnvGroupSavedResponse, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }
    if !allowed.may_deploy() {
        return Err(
            "only somebody who may deploy in this project can change what its services share"
                .into(),
        );
    }

    SetEnvGroup {
        project,
        name,
        env: &request.env,
    }
    .run(database)
    .await
}
