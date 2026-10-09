use wabot::sqlite::SqliteDatabase;

use crate::authz::tenant_user;
use crate::deploy::Deployer;
use crate::entities::account::Account;
use crate::entities::project::Project;
use crate::features::projects::delete_project_command::DeleteProject;
use crate::values::roles::Access;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    allowed: &Access,
    confirm: &str,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to delete".into());
    }

    if !allowed.may_administer() {
        return Err("only an owner can delete this project".into());
    }

    DeleteProject { project, confirm }
        .run(database, deployer)
        .await
}
