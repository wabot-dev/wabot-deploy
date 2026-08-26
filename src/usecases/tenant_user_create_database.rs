use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::create_database::{Asked, CreateDatabase};
use crate::platform::access;
use crate::platform::projects::Project;
use crate::platform::services::Service;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: Asked<'_>,
) -> Result<(Project, Service), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }

    let Some((project, allowed)) = access::find_project(database, account, asked.project)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such project".into());
    };
    if !allowed.may_deploy() {
        return Err("you cannot deploy in this project".into());
    }

    let service = CreateDatabase {
        project_id: &project.id,
        name: asked.name,
        version: asked.version,
        memory: asked.memory,
    }
    .run(database)
    .await?;

    Ok((project, service))
}
