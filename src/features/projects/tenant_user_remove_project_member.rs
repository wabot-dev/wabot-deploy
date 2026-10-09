use wabot::sqlite::SqliteDatabase;

use crate::authz::tenant_user;
use crate::entities::account::Account;
use crate::entities::project::Project;
use crate::features::projects::remove_project_member_command::RemoveProjectMember;
use crate::values::roles::Access;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    person: &str,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    if !allowed.may_administer() {
        return Err("only an owner can change who is here".into());
    }

    RemoveProjectMember {
        project_id: &project.id,
        account_id: person,
    }
    .run(database)
    .await
}
