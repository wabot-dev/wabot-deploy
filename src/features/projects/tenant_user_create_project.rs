use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::features::projects::create_project_command::CreateProject;

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
