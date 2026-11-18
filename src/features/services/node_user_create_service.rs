use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::capability::Capability;
use crate::features::services::create_service_request::CreateServiceRequest as Asked;
use crate::features::services::service_created_response::ServiceCreatedResponse;
use crate::infra::deploy::{quotas, Deployer};
use crate::infra::network;
use crate::infra::repositories::access;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    asked: &Asked,
    env: &[(String, String)],
) -> Result<ServiceCreatedResponse, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }

    let Some((project, allowed)) = access::find_project(database, account, &asked.project)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such project".into());
    };
    if !allowed.may_deploy() {
        return Err("you may look at this project, not change it".into());
    }

    let first_on = match asked.on.trim() {
        "" => None,
        id => match network::candidates_for(database, Capability::Host)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|node| node.id == id)
        {
            Some(node) if node.is_self => None,
            Some(node) => Some(node),
            None => return Err("that node has not agreed to run containers for this one".into()),
        },
    };

    let unborn =
        crate::domain::entities::service::Service::new(&project.id, &asked.name, &asked.image, env)
            .map_err(|error| error.to_string())?;
    if let Some(reason) = quotas::admits(
        database,
        &project,
        &unborn,
        Some(asked.ceilings.memory),
        Some(asked.ceilings.cpu_millicores),
        None,
        1,
    )
    .await
    .map_err(|error| error.to_string())?
    {
        return Err(reason);
    }
    if let Some(reason) = deployer
        .room_for_one_more(first_on.as_ref(), Some(asked.ceilings.memory))
        .await
    {
        return Err(reason);
    }

    let service = crate::infra::repositories::services::create_on(
        database,
        &project.id,
        &asked.name,
        &asked.image,
        env,
        first_on.as_ref().map(|node| node.id.as_str()),
        asked.ceilings,
    )
    .await
    .map_err(|error| error.to_string())?;
    crate::infra::repositories::allowlists::set(
        database,
        &crate::domain::values::allowlist::Door::Service(service.id.clone()),
        &asked.allowlist,
    )
    .await
    .map_err(|error| error.to_string())?;

    Ok(ServiceCreatedResponse {
        service_id: service.id,
        project_slug: project.slug,
        service_slug: service.slug,
    })
}
