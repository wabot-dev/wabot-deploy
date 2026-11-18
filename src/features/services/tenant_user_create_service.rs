use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
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
        .room_for_first_copy(first_on.as_ref(), &unborn, asked.ceilings.memory)
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

#[cfg(test)]
mod tests {
    use crate::features::services::create_service_request::CreateServiceRequest;
    use crate::infra::repositories::{projects, replicas, services};

    async fn a_node(
        database: &wabot::sqlite::SqliteDatabase,
        id: &str,
        overlay_ip: &str,
        runs_containers: bool,
    ) {
        crate::infra::network::save(
            database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: id.into(),
                name: format!("{id}.example"),
                kind: crate::infra::network::Kind::Private,
                endpoint: None,
                public_key: Some("k".into()),
                overlay_ip: Some(overlay_ip.into()),
                is_self: false,
                last_seen_at: None,
                allows: match runs_containers {
                    true => vec![crate::domain::values::capability::Capability::Host],
                    false => Vec::new(),
                },
                ca_pem: None,
            },
        )
        .await
        .expect("node");
    }

    /// An organisation chooses where its service runs from the moment it
    /// makes it, and nothing is started here first: the first copy is
    /// born on that node.
    #[tokio::test]
    async fn an_organisation_makes_a_service_on_a_node_it_picks() {
        let organisation = crate::features::fixtures::organisation().await;
        let database = &organisation.database;
        let project = projects::create_for(
            database,
            "Acme site",
            organisation.admin.tenant_id.as_deref(),
        )
        .await
        .expect("acme's project");
        a_node(database, "nd-far", "10.42.0.8", true).await;
        a_node(database, "nd-shy", "10.42.0.9", false).await;
        let asked = |name: &str, on: &str| CreateServiceRequest {
            project: project.slug.clone(),
            name: name.into(),
            image: String::new(),
            env: String::new(),
            on: on.into(),
            ceilings: Default::default(),
            allowlist: Default::default(),
        };
        let deployer = crate::infra::deploy::Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::infra::config::Config::default(),
        );

        let made = super::run(
            database,
            &deployer,
            &organisation.admin,
            &asked("api", "nd-far"),
            &[],
        )
        .await
        .expect("made there");
        let copies = replicas::of_service(database, &made.service_id)
            .await
            .expect("replicas");
        assert_eq!(copies.len(), 1);
        assert_eq!(copies[0].node_id.as_deref(), Some("nd-far"));

        let here = super::run(
            database,
            &deployer,
            &organisation.admin,
            &asked("web", ""),
            &[],
        )
        .await
        .expect("made here");
        assert!(replicas::of_service(database, &here.service_id)
            .await
            .expect("replicas")
            .iter()
            .all(|replica| replica.is_here()));

        let refused = super::run(
            database,
            &deployer,
            &organisation.admin,
            &asked("worker", "nd-shy"),
            &[],
        )
        .await
        .expect_err("that node never agreed to run containers");
        assert!(refused.contains("agreed"), "{refused}");
        assert!(
            services::in_project(database, &project.id, "worker")
                .await
                .expect("query")
                .is_none(),
            "a refusal left a service behind"
        );
    }

    /// Memory is never promised past what a node may allocate, and a
    /// service is a promise from the moment it exists — so the first copy
    /// is asked about like every one after it, before any row is written.
    #[tokio::test]
    async fn a_service_that_does_not_fit_where_it_would_run_is_not_made() {
        const MB: u64 = 1024 * 1024;
        let organisation = crate::features::fixtures::organisation().await;
        let database = &organisation.database;
        let project = projects::create_for(
            database,
            "Acme site",
            organisation.admin.tenant_id.as_deref(),
        )
        .await
        .expect("acme's project");
        crate::infra::network::save(
            database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: Some(crate::infra::network::Capacity {
                    memory_allocatable: 512 * MB,
                    memory_committed: 384 * MB,
                    cpu_allocatable: 1_000,
                    cpu_committed: 4_000,
                    unbounded: 0,
                }),
                id: "nd-full".into(),
                name: "nd-full.example".into(),
                kind: crate::infra::network::Kind::Private,
                endpoint: None,
                public_key: Some("k".into()),
                overlay_ip: Some("10.42.0.7".into()),
                is_self: false,
                last_seen_at: None,
                allows: vec![crate::domain::values::capability::Capability::Host],
                ca_pem: None,
            },
        )
        .await
        .expect("node");
        let deployer = crate::infra::deploy::Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::infra::config::Config::default(),
        );
        let asked = |name: &str, memory: u64| CreateServiceRequest {
            project: project.slug.clone(),
            name: name.into(),
            image: String::new(),
            env: String::new(),
            on: "nd-full".into(),
            ceilings: crate::domain::entities::service::Ceilings {
                memory,
                cpu_millicores: 2_000,
            },
            allowlist: Default::default(),
        };

        let refused = super::run(
            database,
            &deployer,
            &organisation.admin,
            &asked("big", 256 * MB),
            &[],
        )
        .await
        .expect_err("128 MB left and 256 MB asked");
        assert!(refused.contains("nd-full.example"), "{refused}");
        assert!(
            services::in_project(database, &project.id, "big")
                .await
                .expect("query")
                .is_none(),
            "a refusal left a service behind"
        );

        let made = super::run(
            database,
            &deployer,
            &organisation.admin,
            &asked("small", 128 * MB),
            &[],
        )
        .await
        .expect("what fits is made, and its CPU may be promised past the node's");
        let row = services::find(database, &made.service_id)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(row.memory_limit, Some(128 * MB));
        assert_eq!(row.cpu_millicores, Some(2_000));

        crate::infra::repositories::tenants::set_quota(
            database,
            organisation.admin.tenant_id.as_deref().expect("a tenant"),
            Some(192 * MB),
            None,
            None,
        )
        .await
        .expect("a budget");
        let over_budget = super::run(
            database,
            &deployer,
            &organisation.admin,
            &asked("next", 128 * MB),
            &[],
        )
        .await
        .expect_err("the organisation's budget is the first refusal");
        assert!(over_budget.starts_with("Acme"), "{over_budget}");
    }
}
