use crate::domain::entities::project::Project;
use crate::domain::entities::replica::Replica;
use crate::domain::entities::service::Service;
use crate::domain::policies::presets;
use crate::infra::repositories::{projects, replicas, services};
use crate::infra::runtime::client::Containerd;
use crate::infra::runtime::{ceiling, containers};

use super::Deployer;

/// Every service this node administers that still has no memory ceiling
/// is given one from what its copies were seen to use, and the copies
/// running here take it at once, without a restart.
///
/// Before `reconcile`, because a container `reconcile` replaces takes its
/// peak with it. A copy on another node is measured by what that node
/// last reported and gets the ceiling the way any change reaches it —
/// with a deployment there. Not admitted against the node: what is
/// already running cannot be refused, and a sum past what the node may
/// promise is said on its page and refuses the next promise instead.
pub async fn give_every_service_one(deployer: &Deployer) -> usize {
    let (Ok(services), Ok(projects)) = (
        services::all(&deployer.database, None).await,
        projects::all(&deployer.database).await,
    ) else {
        return 0;
    };
    let client = Containerd::connect().await.ok();
    let mut given = 0;
    for service in services
        .iter()
        .filter(|service| service.memory_limit.is_none() && service.origin_node_id.is_none())
    {
        let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
            continue;
        };
        let Ok(copies) = replicas::of_service(&deployer.database, &service.id).await else {
            continue;
        };
        let copies: Vec<Replica> = copies.into_iter().filter(|c| !c.evicted()).collect();
        let running = running_here(client.as_ref(), project, service, &copies).await;
        let peak = running
            .iter()
            .filter_map(|(_, pid)| ceiling::peak(*pid))
            .chain(
                copies
                    .iter()
                    .filter(|c| !c.is_here())
                    .filter_map(|c| c.memory_bytes),
            )
            .max();
        let memory = presets::memory_from_peak(peak);

        if let Err(error) =
            services::set_memory_limit(&deployer.database, &service.id, Some(memory)).await
        {
            tracing::warn!(service = %service.slug, %error, "could not give it a memory ceiling");
            continue;
        }
        for (container, pid) in &running {
            if let Err(error) = ceiling::bound(*pid, memory) {
                tracing::warn!(%container, %error, "it takes its ceiling at its next deployment");
            }
        }
        if copies.iter().any(|copy| !copy.is_here()) {
            if let Ok(Some(fresh)) = services::find(&deployer.database, &service.id).await {
                deployer
                    .tell_holders_including(project, &fresh, &Default::default())
                    .await;
            }
        }
        tracing::info!(
            service = %service.slug,
            ceiling = %presets::label(memory),
            peak = %peak.map_or("nothing".to_string(), crate::infra::node::memory::human),
            "given a memory ceiling from what it used"
        );
        given += 1;
    }
    given
}

async fn running_here(
    client: Option<&Containerd>,
    project: &Project,
    service: &Service,
    copies: &[Replica],
) -> Vec<(String, u32)> {
    let Some(client) = client else {
        return Vec::new();
    };
    let mut running = Vec::new();
    for copy in copies.iter().filter(|copy| copy.is_here()) {
        let id = copy.container_id(&project.slug, &service.slug);
        if let Ok(Some(task)) = containers::status(client, &id).await {
            if task.running() && task.pid > 0 {
                running.push((id, task.pid));
            }
        }
    }
    running
}

#[cfg(test)]
mod tests {
    use crate::infra::repositories::{projects, replicas, services};

    const MB: u64 = 1024 * 1024;

    async fn a_node(database: &wabot::sqlite::SqliteDatabase, id: &str) {
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
                overlay_ip: Some("10.42.0.8".into()),
                is_self: false,
                last_seen_at: None,
                allows: vec![crate::domain::values::capability::Capability::Host],
                ca_pem: None,
            },
        )
        .await
        .expect("node");
    }

    async fn without_a_ceiling(
        database: &wabot::sqlite::SqliteDatabase,
        project_id: &str,
        name: &str,
        on: Option<&str>,
    ) -> crate::domain::entities::service::Service {
        let service = services::create_on(
            database,
            project_id,
            name,
            "docker.io/library/nginx:alpine",
            &[],
            on,
            Default::default(),
        )
        .await
        .expect("service");
        services::set_memory_limit(database, &service.id, None)
            .await
            .expect("as it was before every service was born with one");
        service
    }

    /// The case that took a node down: a service with no ceiling, which
    /// the node's arithmetic counted as nothing.
    #[tokio::test]
    async fn a_service_without_a_ceiling_is_given_one_from_what_it_was_seen_to_use() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        a_node(&database, "nd-far").await;
        let project = projects::create(&database, "shop").await.expect("project");
        let far = without_a_ceiling(&database, &project.id, "far", Some("nd-far")).await;
        let copy = &replicas::of_service(&database, &far.id)
            .await
            .expect("copies")[0];
        replicas::set_memory(&database, &copy.id, Some(300 * MB))
            .await
            .expect("what that node reported");
        let idle = without_a_ceiling(&database, &project.id, "idle", None).await;
        let chosen = services::create(&database, &project.id, "chosen", "", &[])
            .await
            .expect("born with one");
        services::set_memory_limit(&database, &chosen.id, Some(64 * MB))
            .await
            .expect("chosen by somebody");

        let deployer = super::Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::infra::config::Config::default(),
        );
        assert_eq!(super::give_every_service_one(&deployer).await, 2);

        let ceiling = |id: String| {
            let database = database.clone();
            async move {
                services::find(&database, &id)
                    .await
                    .expect("query")
                    .expect("row")
                    .memory_limit
            }
        };
        assert_eq!(ceiling(far.id).await, Some(512 * MB));
        assert_eq!(
            ceiling(idle.id).await,
            Some(crate::domain::policies::presets::DEFAULT_MEMORY),
            "nothing measured is the default"
        );
        assert_eq!(
            ceiling(chosen.id).await,
            Some(64 * MB),
            "a ceiling somebody chose is not second-guessed"
        );
        assert_eq!(
            super::give_every_service_one(&deployer).await,
            0,
            "and it is given once"
        );
    }

    #[tokio::test]
    async fn a_copy_another_node_placed_here_is_that_node_to_bound() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        a_node(&database, "nd-owner").await;
        let project = projects::create(&database, "shop").await.expect("project");
        let placed = without_a_ceiling(&database, &project.id, "placed", None).await;
        services::set_origin(&database, &placed.id, "nd-owner")
            .await
            .expect("placed here by another node");

        let deployer = super::Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::infra::config::Config::default(),
        );
        assert_eq!(super::give_every_service_one(&deployer).await, 0);
        assert_eq!(
            services::find(&database, &placed.id)
                .await
                .expect("query")
                .expect("row")
                .memory_limit,
            None
        );
    }
}
