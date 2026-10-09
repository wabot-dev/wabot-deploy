use wabot::sqlite::SqliteDatabase;

use crate::authz::node_user;
use crate::entities::account::Account;
use crate::entities::project::Project;
use crate::features::projects::create_project_command::CreateProject;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    name: &str,
) -> Result<Project, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }

    CreateProject { name, by: account }.run(database).await
}
