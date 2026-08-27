use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::create_project::CreateProject;
use crate::entities::project::Project;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    name: &str,
) -> Result<Project, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }

    CreateProject { name, by: account }.run(database).await
}
