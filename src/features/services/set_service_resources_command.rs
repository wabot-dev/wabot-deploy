use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::infra::deploy::Deployer;

use crate::domain::entities::database::Engine;
use crate::domain::entities::service::Service;
use crate::domain::policies::presets;
use crate::infra::deploy::quotas;
use crate::infra::repositories::replicas;
use crate::infra::repositories::services;

pub struct SetServiceResources<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    pub asked:
        &'a crate::features::services::set_service_resources_request::SetServiceResourcesRequest,
}

impl SetServiceResources<'_> {
    /// **The organisation's ceiling first, then the node's.** The tenant
    /// sum is local rows and can always be answered, it is the more
    /// general refusal — no node helps when there is no quota left — and
    /// a refusal should lead to the smallest correction.
    pub async fn run(
        &self,
        database: &SqliteDatabase,
        deployer: &Deployer,
    ) -> Result<Service, String> {
        // A managed engine's ceiling is a preset, because the number also
        // sets the engine's arithmetic; anything else takes a size.
        let memory = match self.service.kind.is_managed() {
            true => presets::parse(&self.asked.memory),
            false => presets::parse_size(&self.asked.memory),
        }?
        .ok_or("a service keeps a memory ceiling: the node promises it and never more")?;
        let memory = Some(memory);
        // Keeping a size it already has is not choosing it: a database
        // made at 64 MB before the floor would otherwise refuse every
        // change to its cores until somebody also raised its memory.
        if let (Some(engine), Some(asked)) = (self.service.kind.engine(), memory) {
            if asked < engine.floor() && memory != self.service.memory_limit {
                return Err(match engine {
                    Engine::Mysql => format!(
                        "MySQL is killed below {} however it is tuned, so that is the smallest \
                         size it takes",
                        presets::label(engine.floor())
                    ),
                    Engine::Valkey => format!(
                        "{} is the smallest size Valkey takes: an idle server and the room its \
                         writes need take most of anything less",
                        presets::label(engine.floor())
                    ),
                    _ => format!(
                        "{} is the smallest size PostgreSQL takes: in less, the kernel can kill \
                         initdb before it finishes, and a database is not given a size it could \
                         not have been made in",
                        presets::label(engine.floor())
                    ),
                });
            }
        }
        let cpu =
            Some(presets::parse_cores(&self.asked.cpu)?.ok_or("a service keeps a CPU ceiling")?);
        let disk = presets::parse_size(&self.asked.disk)?;
        if self.service.kind.is_store() {
            self.fits_a_store(database, memory, disk).await?;
        }

        let copies = replicas::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|replica| !replica.evicted())
            .count()
            .max(1) as u64;

        // A stopped service holds nothing, so its memory and CPU are asked
        // about when it starts; its disk is still there.
        let promises = self.service.promises();
        if let Some(reason) = quotas::admits(
            database,
            self.project,
            self.service,
            memory.filter(|_| promises),
            cpu.filter(|_| promises),
            disk,
            copies,
        )
        .await
        .map_err(|error| error.to_string())?
        {
            return Err(reason);
        }
        if promises {
            if let Some(reason) = deployer.room_for_every_copy(self.service, memory).await {
                return Err(reason);
            }
        }

        for write in [
            services::set_memory_limit(database, &self.service.id, memory).await,
            services::set_cpu_limit(database, &self.service.id, cpu).await,
            services::set_disk_limit(database, &self.service.id, disk).await,
        ] {
            write.map_err(|error| error.to_string())?;
        }

        Ok(services::find(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .unwrap_or_else(|| self.service.clone()))
    }

    /// A store's disk is what its buckets' maximums add up against, so it
    /// keeps one and it does not go under what they already hold.
    async fn fits_a_store(
        &self,
        database: &SqliteDatabase,
        memory: Option<u64>,
        disk: Option<u64>,
    ) -> Result<(), String> {
        let floor = crate::domain::policies::garage::FLOOR;
        if memory.is_some_and(|asked| asked < floor) && memory != self.service.memory_limit {
            return Err(format!(
                "{} is the smallest size a store takes",
                presets::label(floor)
            ));
        }
        let held = crate::infra::repositories::object_stores::reserved(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?;
        match disk {
            None => {
                Err("a store's buckets are sized against its disk, so it keeps a disk size".into())
            }
            Some(disk) if disk < held => Err(format!(
                "its buckets may hold up to {} together, so its disk cannot be {} — make a \
                 bucket smaller or delete one first",
                presets::size_field(held),
                presets::size_field(disk)
            )),
            Some(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::service::DesiredState;

    /// A stopped service holds nothing, so its ceiling is asked about
    /// when it starts and not before: a node promised past what it has
    /// would otherwise refuse a change to something it is not running.
    #[tokio::test]
    async fn a_stopped_service_takes_any_ceiling_and_a_running_one_has_to_fit() {
        const MB: u64 = 1024 * 1024;
        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::network::save(
            &database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: Some(crate::infra::network::Capacity {
                    memory_allocatable: 512 * MB,
                    memory_committed: 700 * MB,
                    cpu_allocatable: 1_000,
                    cpu_committed: 0,
                    unbounded: 0,
                }),
                id: "nd-l1".into(),
                name: "nd-l1.example".into(),
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
        let project = crate::infra::repositories::projects::create(&database, "shared")
            .await
            .expect("project");
        let service = services::create(
            &database,
            &project.id,
            "web",
            "hub.example/web@sha256:a",
            &[],
        )
        .await
        .expect("service");
        let copy = replicas::of_service(&database, &service.id)
            .await
            .expect("copies")
            .remove(0);
        replicas::move_to(&database, &copy.id, Some("nd-l1"))
            .await
            .expect("moved");
        let deployer = Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::infra::config::Config::default(),
        );
        let asked = |memory: &str| {
            crate::features::services::set_service_resources_request::SetServiceResourcesRequest {
                memory: memory.into(),
                cpu: "0.5".into(),
                disk: String::new(),
            }
        };
        let set = |service: Service, memory: &'static str| {
            let (database, deployer, project) = (&database, &deployer, &project);
            async move {
                SetServiceResources {
                    project,
                    service: &service,
                    asked: &asked(memory),
                }
                .run(database, deployer)
                .await
            }
        };

        let refused = set(service.clone(), "256 MB")
            .await
            .expect_err("running, on a node promised past what it has");
        assert!(refused.starts_with("nd-l1.example has"), "{refused}");
        set(service.clone(), "64 MB").await.expect("lowered");

        services::set_desired_state(&database, &service.id, DesiredState::Stopped)
            .await
            .expect("stopped");
        let stopped = services::find(&database, &service.id)
            .await
            .expect("query")
            .expect("row");
        let saved = set(stopped, "512 MB").await.expect("stopped holds nothing");
        assert_eq!(saved.memory_limit, Some(512 * MB));
    }
}
