use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::create_service::{Asked, CreateService};
use crate::entities::service::Service;
use crate::platform::access;
use crate::platform::projects::Project;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: Asked<'_>,
    env: &[(String, String)],
) -> Result<(Project, Service), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }

    let Some((project, allowed)) = access::find_project(database, account, asked.project)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such project".into());
    };
    if !allowed.may_deploy() {
        return Err("you may look at this project, not change it".into());
    }

    let service = CreateService {
        project_id: &project.id,
        name: asked.name,
        image: asked.image,
        env,
    }
    .run(database)
    .await
    .map_err(|error| error.to_string())?;

    Ok((project, service))
}
