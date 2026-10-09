use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::domain::values::roles::ProjectRole;
use crate::features::projects::add_project_member_command::AddProjectMember;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    username: &str,
    role: ProjectRole,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    if !allowed.may_administer() {
        return Err("only an owner can change who is here".into());
    }

    AddProjectMember {
        project_id: &project.id,
        username,
        role,
    }
    .run(database)
    .await
}
