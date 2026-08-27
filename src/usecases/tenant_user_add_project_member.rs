use wabot::sqlite::SqliteDatabase;

use crate::accounts::roles::Access;
use crate::accounts::roles::ProjectRole;
use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::add_project_member::AddProjectMember;
use crate::entities::project::Project;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    username: &str,
    role: ProjectRole,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
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
