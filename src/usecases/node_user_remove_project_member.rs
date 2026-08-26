use wabot::sqlite::SqliteDatabase;

use crate::accounts::roles::Access;
use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::remove_project_member::RemoveProjectMember;
use crate::platform::projects::Project;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    person: &str,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
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
