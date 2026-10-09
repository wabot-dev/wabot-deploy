use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::create_project_token_command::CreateProjectToken;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    name: &str,
) -> Result<String, String> {
    if !tenant_user::belongs_here(account) {
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
