//! Make a managed database: the service, its ceiling, its volume, its
//! port and its row — in that order, because each one needs the last.

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::database::{Database, Engine, Mode};
use crate::domain::entities::service::Service;
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::domain::policies::presets;
use crate::domain::values::image_reference::ImageReference;
use crate::infra::clock::now_ms;
use crate::infra::repositories::databases::REPLICATION_USER;

/// An unbounded Postgres on a node with one core is the node, so the
/// form's empty answer is the form's default answer rather than "no
/// ceiling".
pub const DEFAULT_PRESET: u64 = 256 * 1024 * 1024;

pub struct CreateDatabase<'a> {
    pub project_id: &'a str,
    pub name: &'a str,
    pub engine: Engine,
    pub mode: Mode,
    pub version: &'a str,
    pub memory_limit: u64,
    pub image: Option<&'a str>,
    /// The node its primary runs on, by id; `None` for this one. Fixed
    /// from here on: nothing moves a primary, because moving one that
    /// holds data is a switchover nobody has built.
    pub on: Option<&'a str>,
}

impl CreateDatabase<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> PlatformResult<(Service, Database)> {
        let engine = self.engine;
        if !Engine::OFFERED.contains(&engine) {
            return Err(PlatformError::Refused(
                "this node does not offer that engine".into(),
            ));
        }
        if !engine.versions().contains(&self.version) {
            return Err(PlatformError::Refused(format!(
                "this node does not offer {} {}",
                engine.label(),
                self.version
            )));
        }
        if self.mode == Mode::Cache && engine != Engine::Valkey {
            return Err(PlatformError::Refused(format!(
                "{} keeps what it holds; only Valkey can be made as a cache",
                engine.label()
            )));
        }
        if !presets::LADDER.contains(&self.memory_limit) {
            return Err(PlatformError::Refused(
                "that is not one of the sizes on offer".into(),
            ));
        }
        if self.memory_limit < engine.floor() {
            return Err(PlatformError::Refused(match engine {
                Engine::Mysql => format!(
                    "MySQL is not started in {}: the kernel kills it while it is still \
                     initialising, however it is tuned — choose {} or more",
                    presets::label(self.memory_limit),
                    presets::label(engine.floor())
                ),
                Engine::Valkey => format!(
                    "Valkey is not started in {}: an idle server and the room its writes \
                     need take most of it — choose {} or more",
                    presets::label(self.memory_limit),
                    presets::label(engine.floor())
                ),
                _ => format!(
                    "a database is not started in {}: initdb writes its catalogs through the \
                     page cache, which the ceiling counts, and the kernel can stop it before it \
                     finishes — choose {} or more",
                    presets::label(self.memory_limit),
                    presets::label(engine.floor())
                ),
            }));
        }
        let image = match self.image {
            None => engine.image_for(self.version),
            Some(typed) => {
                let reference = ImageReference::typed(typed)?;
                if reference.is_empty() {
                    return Err(PlatformError::Refused(format!(
                        "a custom image needs a reference — the one this node uses for {} {} \
                         is {}",
                        engine.label(),
                        self.version,
                        engine.image_for(self.version)
                    )));
                }
                reference.to_string()
            }
        };

        if let Some(node) = self.on {
            let Some(row) = crate::infra::network::find(database, node)
                .await
                .map_err(|error| PlatformError::Refused(error.to_string()))?
            else {
                return Err(PlatformError::Refused("no such node".into()));
            };
            if !row
                .allows
                .contains(&crate::domain::values::capability::Capability::Store)
            {
                return Err(PlatformError::Refused(format!(
                    "{} has not agreed to keep data for this node",
                    row.name
                )));
            }
            if engine == Engine::Valkey
                && !crate::infra::network::errand::knows_valkey(row.version.as_deref())
            {
                return Err(PlatformError::Refused(format!(
                    "{} runs {}, which does not know Valkey — it would refuse the database \
                     and it would wait there for ever. Update it to {} or later first",
                    row.name,
                    row.version
                        .as_deref()
                        .unwrap_or("a version it has not reported"),
                    crate::infra::network::errand::VALKEY_SINCE
                )));
            }
        }

        let service = crate::infra::repositories::services::create_on(
            database,
            self.project_id,
            self.name,
            &image,
            &[],
            self.on,
            crate::domain::entities::service::Ceilings {
                memory: self.memory_limit,
                ..Default::default()
            },
        )
        .await?;

        // Marked before anything else reads it. A service that was a plain
        // container for even a moment is one the deploy path would have
        // started without its volume — which for a database is an empty
        // one that looks like it worked.
        crate::infra::repositories::services::set_kind(database, &service.id, engine.kind())
            .await?;

        // The volume is what makes it a database rather than a container
        // that forgets. Nothing else on the node declares one yet.
        if self.mode.keeps_data() {
            crate::infra::repositories::volumes::create(
                database,
                &service.id,
                crate::domain::policies::postgres::VOLUME,
                engine.data_mount(),
            )
            .await?;
        }

        // Declared and not published: the node has to know what the
        // container listens on to bind anything for it — an overlay port
        // for a standby elsewhere, or a host port if somebody publishes
        // it — and publishing is a separate decision somebody confirms.
        crate::infra::repositories::ports::create(
            database,
            &service.id,
            engine.port(),
            false,
            None,
            &[],
        )
        .await?;

        let identifier = engine.identifier(&service.slug, REPLICATION_USER);
        let row = Database {
            service_id: service.id.clone(),
            engine,
            mode: self.mode,
            version: self.version.to_string(),
            admin_user: identifier.clone(),
            admin_password: wabot::prelude::password::generate(24),
            database_name: identifier,
            // A fixed self.name, because it is this node's own role rather than
            // anything the operator chose, and one less thing that can
            // differ between the primary's `pg_hba.conf` and a standby's
            // conninfo.
            replication_user: REPLICATION_USER.to_string(),
            replication_password: wabot::prelude::password::generate(24),
            primary_slot: 1,
            // Derived here, always: this node owns the database, so it can
            // read where its own primary answers out of its own rows, and
            // self.name it under its own domain.
            primary_endpoint: None,
            owner_domain: None,
            restored_from: None,
            recovery_target: None,
            replication_from: None,
            owner_name: None,
            owner_published: false,
        };

        let stored = row.clone();
        database
            .write(move |connection| {
                connection.execute(
                    "INSERT INTO database \
                       (\"service_id\", \"engine\", \"version\", \"admin_user\", \
                        \"admin_password\", \"database_name\", \"replication_user\", \
                        \"replication_password\", \"primary_slot\", \"created_at\", \"mode\") \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    (
                        stored.service_id,
                        stored.engine.as_str(),
                        stored.version,
                        stored.admin_user,
                        stored.admin_password,
                        stored.database_name,
                        stored.replication_user,
                        stored.replication_password,
                        stored.primary_slot,
                        now_ms(),
                        stored.mode.as_str(),
                    ),
                )?;
                Ok(())
            })
            .await?;

        // The address the connection string names. Allocated now rather
        // than at the first deployment, because the page shows it before
        // anything has started.
        for replica in crate::infra::repositories::replicas::of_service(database, &service.id)
            .await?
            .into_iter()
            .filter(|replica| replica.is_here())
        {
            crate::infra::repositories::replicas::reserve_host(
                database,
                self.project_id,
                &replica.id,
            )
            .await?;
        }

        // Read back: the row carries the kind and the ceiling that were
        // set after it was made, and a caller handed the `create` copy
        // would see neither.
        let service = crate::infra::repositories::services::find(database, &service.id)
            .await?
            .ok_or_else(|| PlatformError::Refused("the service vanished as it was made".into()))?;

        tracing::info!(
            service = %service.slug,
            engine = engine.as_str(),
            mode = self.mode.as_str(),
            version = self.version,
            limit = presets::label(self.memory_limit),
            "created a database"
        );
        Ok((service, row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn far_node(database: &SqliteDatabase, version: Option<&str>) {
        crate::infra::network::ensure_self(database, &crate::infra::config::Config::default())
            .await
            .expect("self");
        crate::infra::network::save(
            database,
            &crate::infra::network::Node {
                update_note: None,
                version: version.map(str::to_string),
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-far".into(),
                name: "far.example".into(),
                kind: crate::infra::network::Kind::Private,
                overlay_ip: Some("10.42.0.9".into()),
                public_key: None,
                endpoint: None,
                allows: vec![crate::domain::values::capability::Capability::Store],
                last_seen_at: None,
                is_self: false,
                ca_pem: None,
            },
        )
        .await
        .expect("node");
    }

    fn valkey<'a>(project_id: &'a str, mode: Mode, on: Option<&'a str>) -> CreateDatabase<'a> {
        CreateDatabase {
            project_id,
            name: "sessions",
            engine: Engine::Valkey,
            mode,
            version: "9.1",
            memory_limit: 64 * 1024 * 1024,
            image: None,
            on,
        }
    }

    /// A node older than the engine reads the row as one it does not know,
    /// so the copy would be sent, refused there, and wait for ever on a
    /// page that says it was placed.
    #[tokio::test]
    async fn valkey_is_not_placed_on_a_node_that_does_not_know_it() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");

        for version in [None, Some("0.36.2")] {
            far_node(&database, version).await;
            let refused = valkey(&project.id, Mode::Store, Some("nd-far"))
                .run(&database)
                .await
                .expect_err("too old")
                .to_string();
            assert!(
                refused.contains(crate::infra::network::errand::VALKEY_SINCE),
                "{refused}"
            );
        }

        far_node(&database, Some(crate::infra::network::errand::VALKEY_SINCE)).await;
        valkey(&project.id, Mode::Store, Some("nd-far"))
            .run(&database)
            .await
            .expect("a node that knows it");
    }

    #[tokio::test]
    async fn a_cache_is_made_with_nothing_to_keep_data_in_and_a_store_with_a_volume() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");

        let (cache, row) = valkey(&project.id, Mode::Cache, None)
            .run(&database)
            .await
            .expect("a cache");
        assert_eq!(row.mode, Mode::Cache);
        assert!(
            crate::infra::repositories::volumes::of_service(&database, &cache.id)
                .await
                .expect("volumes")
                .is_empty()
        );
        let read = crate::infra::repositories::databases::of_service(&database, &cache.id)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(read.mode, Mode::Cache, "and it reads back as one");

        let (store, _) = CreateDatabase {
            name: "queue",
            ..valkey(&project.id, Mode::Store, None)
        }
        .run(&database)
        .await
        .expect("a store");
        assert_eq!(
            crate::infra::repositories::volumes::of_service(&database, &store.id)
                .await
                .expect("volumes")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn only_valkey_can_be_a_cache() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");
        let refused = CreateDatabase {
            engine: Engine::Postgres,
            version: "17",
            memory_limit: 256 * 1024 * 1024,
            ..valkey(&project.id, Mode::Cache, None)
        }
        .run(&database)
        .await
        .expect_err("a postgres that forgets");
        assert!(refused.to_string().contains("only Valkey"), "{refused}");
    }
}
