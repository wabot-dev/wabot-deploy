use wabot::sqlite::SqliteDatabase;

use crate::accounts::roles::Access;
use crate::accounts::Account;
use crate::authz::node_user;
use crate::entities::project::Project;
use crate::features::projects::revoke_project_token_command::RevokeProjectToken;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    token: &str,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
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
