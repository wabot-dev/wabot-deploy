use wabot::sqlite::SqliteDatabase;

use crate::accounts::roles::Access;
use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::create_project_token::CreateProjectToken;
use crate::entities::project::Project;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    name: &str,
) -> Result<String, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to mint".into());
    }

    if !allowed.may_deploy() {
        return Err("you may look at this project, not push to it".into());
    }

    CreateProjectToken {
        project_id: &project.id,
        name,
        by: &account.id,
    }
    .run(database)
    .await
}
