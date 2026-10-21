use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::delete_project_command::DeleteProject;
use crate::infra::deploy::Deployer;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    allowed: &Access,
    confirm: &str,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to delete".into());
    }

    if !allowed.may_administer() {
        return Err("only an owner can delete this project".into());
    }

    DeleteProject { project, confirm }
        .run(database, container, deployer)
        .await
}
