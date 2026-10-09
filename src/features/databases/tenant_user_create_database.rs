use wabot::sqlite::SqliteDatabase;

use crate::authz::tenant_user;
use crate::entities::account::Account;
use crate::entities::project::Project;
use crate::entities::service::Service;
use crate::features::databases::create_database_request::CreateDatabaseRequest;
use crate::policies::access;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &CreateDatabaseRequest,
) -> Result<(Project, Service), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }

    let Some((project, allowed)) = access::find_project(database, account, &asked.project)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such project".into());
    };
    if !allowed.may_deploy() {
        return Err("you cannot deploy in this project".into());
    }

    // Blank is not "no ceiling" here. An unbounded Postgres on a node
    // with one core is the node, so the form's empty answer is the
    // form's default answer.
    let memory_limit = crate::policies::presets::parse(&asked.memory)?
        .unwrap_or(crate::features::databases::create_database_command::DEFAULT_PRESET);

    let (service, _) = crate::features::databases::create_database_command::CreateDatabase {
        project_id: &project.id,
        name: &asked.name,
        version: &asked.version,
        memory_limit,
    }
    .run(database)
    .await
    .map_err(|error| error.to_string())?;

    Ok((project, service))
}
