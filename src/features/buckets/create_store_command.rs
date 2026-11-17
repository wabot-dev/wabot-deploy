//! Make a store of buckets: the service, its ceilings, its volume, its row
//! and its name — the name last, because it is the one step that asks DNS,
//! and a refusal there takes the rest back.

use sha2::Digest;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::object_store::ObjectStore;
use crate::domain::entities::project::Project;
use crate::domain::entities::service::{Kind, Service};
use crate::domain::policies::{garage, presets};
use crate::features::buckets::create_store_request::CreateStoreRequest;
use crate::features::services::add_service_port_command::AddServicePort;
use crate::features::services::add_service_port_request::AddServicePortRequest;
use crate::infra::repositories::{object_stores, services, volumes};

pub const DEFAULT_PRESET: u64 = 256 * 1024 * 1024;

pub struct CreateStore<'a> {
    pub project: &'a Project,
    pub asked: &'a CreateStoreRequest,
}

struct Sized {
    hostname: String,
    memory_limit: u64,
    disk_limit: u64,
}

impl CreateStore<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Service, String> {
        if !crate::infra::node::settings::buckets_enabled(database).await {
            return Err(garage::OFF.into());
        }
        let memory_limit = presets::parse(&self.asked.memory)?.unwrap_or(DEFAULT_PRESET);
        if !presets::LADDER.contains(&memory_limit) {
            return Err("that is not one of the sizes on offer".into());
        }
        if memory_limit < garage::FLOOR {
            return Err(format!(
                "a store is not started in {}: choose {} or more",
                presets::label(memory_limit),
                presets::label(garage::FLOOR)
            ));
        }
        let disk_limit = presets::parse_bytes(&self.asked.disk)?
            .ok_or("a store needs a disk size: what its buckets may hold together")?;
        let hostname = match self.asked.hostname.trim() {
            "" => self.derived(database).await.ok_or(
                "this node has no domain to derive a name under, so the store needs one typed",
            )?,
            typed => typed.to_string(),
        };
        if crate::domain::values::hostname::Hostname::parse(&hostname).is_wildcard() {
            return Err("a store answers to one name, and a wildcard is many".into());
        }
        let sized = Sized {
            hostname,
            memory_limit,
            disk_limit,
        };
        let image = garage::image_for(garage::DEFAULT_VERSION)
            .ok_or("this node offers no version of the store's engine")?;
        let refused = |error: crate::domain::errors::PlatformError| error.to_string();

        let service = services::create_on(
            database,
            &self.project.id,
            &self.asked.name,
            &image,
            &[],
            None,
        )
        .await
        .map_err(refused)?;
        let made = self.fill(database, &service, &sized).await;
        if let Err(reason) = made {
            if let Err(error) = services::delete(database, &service.id).await {
                tracing::error!(service = %service.slug, %error, "could not take a refused store back");
            }
            return Err(reason);
        }

        let service = services::find(database, &service.id)
            .await
            .map_err(refused)?
            .ok_or("the store vanished as it was made")?;
        tracing::info!(
            service = %service.slug,
            memory = presets::label(sized.memory_limit),
            disk = presets::label(sized.disk_limit),
            "created a store of buckets"
        );
        Ok(service)
    }

    async fn derived(&self, database: &SqliteDatabase) -> Option<String> {
        let domain = crate::infra::node::settings::domain(database).await;
        let suffix = crate::infra::repositories::project_domain::domain_for(
            database,
            self.project,
            domain.as_deref(),
        )
        .await?;
        Some(crate::infra::deploy::dns::suggested_hostname(
            crate::domain::values::slug::Slug::of(&self.asked.name).as_str(),
            &self.project.slug,
            &suffix,
        ))
    }

    async fn fill(
        &self,
        database: &SqliteDatabase,
        service: &Service,
        sized: &Sized,
    ) -> Result<(), String> {
        let refused = |error: crate::domain::errors::PlatformError| error.to_string();
        services::set_kind(database, &service.id, Kind::Garage)
            .await
            .map_err(refused)?;
        services::set_memory_limit(database, &service.id, Some(sized.memory_limit))
            .await
            .map_err(refused)?;
        services::set_disk_limit(database, &service.id, Some(sized.disk_limit))
            .await
            .map_err(refused)?;
        volumes::create(
            database,
            &service.id,
            garage::DATA_VOLUME,
            garage::DATA_MOUNT,
        )
        .await
        .map_err(refused)?;
        object_stores::create(
            database,
            &ObjectStore {
                service_id: service.id.clone(),
                version: garage::DEFAULT_VERSION.to_string(),
                rpc_secret: rpc_secret(),
                admin_token: wabot::prelude::password::generate(40),
            },
        )
        .await
        .map_err(refused)?;
        AddServicePort {
            service,
            asked: &AddServicePortRequest {
                container_port: garage::S3_PORT,
                publish: false,
                hostname: sized.hostname.clone(),
                https: true,
                without_authority: false,
            },
        }
        .run(database)
        .await?;
        crate::infra::repositories::allowlists::set(
            database,
            &crate::domain::values::allowlist::Door::Service(service.id.clone()),
            &self.asked.allowlist,
        )
        .await
        .map_err(refused)
    }
}

fn rpc_secret() -> String {
    sha2::Sha256::digest(wabot::prelude::password::generate(48).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn project() -> (SqliteDatabase, Project) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");
        (database, project)
    }

    fn files() -> CreateStoreRequest {
        CreateStoreRequest {
            project: "shop".into(),
            name: "files".into(),
            hostname: "files.shop.example.com".into(),
            memory: String::new(),
            disk: "10 GB".into(),
            allowlist: Default::default(),
        }
    }

    async fn made(
        database: &SqliteDatabase,
        project: &Project,
        asked: &CreateStoreRequest,
    ) -> Result<Service, String> {
        CreateStore { project, asked }.run(database).await
    }

    #[tokio::test]
    async fn a_store_is_refused_while_the_node_has_not_turned_them_on() {
        let (database, project) = project().await;
        let refused = made(&database, &project, &files()).await.expect_err("off");
        assert_eq!(refused, garage::OFF);
        assert!(services::all(&database, Some(&project.id))
            .await
            .expect("services")
            .is_empty());
    }

    #[tokio::test]
    async fn a_store_under_its_floor_is_refused_with_the_size_it_takes() {
        let (database, project) = project().await;
        crate::infra::node::settings::set_buckets_enabled(&database, true)
            .await
            .expect("on");
        let asked = CreateStoreRequest {
            memory: (64 * 1024 * 1024).to_string(),
            ..files()
        };
        let refused = made(&database, &project, &asked)
            .await
            .expect_err("too small");
        assert!(
            refused.contains(&presets::label(garage::FLOOR)),
            "{refused}"
        );
    }

    /// Without a name checked against DNS, the store would be a service
    /// nothing routes to, and the half-made rows would hold its slug.
    #[tokio::test]
    async fn a_name_dns_refuses_takes_the_whole_store_back() {
        let (database, project) = project().await;
        crate::infra::node::settings::set_buckets_enabled(&database, true)
            .await
            .expect("on");
        made(&database, &project, &files())
            .await
            .expect_err("no node answers for names in a test");
        assert!(services::all(&database, Some(&project.id))
            .await
            .expect("services")
            .is_empty());
    }

    #[test]
    fn the_secret_is_what_garage_reads_and_differs_each_time() {
        let (one, two) = (rpc_secret(), rpc_secret());
        assert!(garage::is_rpc_secret(&one), "{one}");
        assert_ne!(one, two);
    }
}
