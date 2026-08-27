use wabot::sqlite::SqliteDatabase;

use crate::accounts::roles::Access;
use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::delete_project::DeleteProject;
use crate::deploy::Deployer;
use crate::entities::project::Project;

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
