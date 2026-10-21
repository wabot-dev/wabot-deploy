use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::infra::deploy::Deployer;
use crate::infra::repositories::tenants;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    deployer: &Deployer,
    account: &Account,
    organisation: &str,
) -> Result<(), String> {
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let Some(tenant) = tenants::find(database, organisation)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such organisation".into());
    };

    // Each project the way deleting it from its own page does, so every
    // service is torn down and its storage discarded rather than its rows
    // dropped. The card always said this; the delete refused instead.
    let projects = crate::infra::repositories::access::of_tenant(database, &tenant.id)
        .await
        .map_err(|error| error.to_string())?;
    for project in &projects {
        crate::features::projects::delete_project_command::DeleteProject {
            project,
            confirm: &project.slug,
        }
        .run(database, container, deployer)
        .await?;
    }

    tenants::delete(database, &tenant.id)
        .await
        .map_err(|error| error.to_string())?;

    // Its route goes with it, or the name keeps answering for a frontier
    // that no longer exists.
    deployer.sync_routes().await;
    Ok(())
}
