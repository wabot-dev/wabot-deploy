use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::create_env_group_command::CreateEnvGroup;
use crate::features::projects::create_env_group_request::CreateEnvGroupRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    request: &CreateEnvGroupRequest,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }
    if !allowed.may_deploy() {
        return Err(
            "only somebody who may deploy in this project can change what its services share"
                .into(),
        );
    }

    CreateEnvGroup {
        project,
        name: &request.name,
        env: &request.env,
    }
    .run(database)
    .await
}
