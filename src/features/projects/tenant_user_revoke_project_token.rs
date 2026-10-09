use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::revoke_project_token_command::RevokeProjectToken;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    token: &str,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to revoke".into());
    }

    if !allowed.may_deploy() {
        return Err("you may look at this project, not push to it".into());
    }

    RevokeProjectToken {
        project_id: &project.id,
        token_id: token,
    }
    .run(database)
    .await
}
