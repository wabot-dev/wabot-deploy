//! Make a managed database: the service, its ceiling, its volume, its
//! port and its row — in that order, because each one needs the last.

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::database::{identifier, Database, Engine};
use crate::domain::entities::service::{Kind, Service};
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::domain::policies::{postgres, presets};
use crate::infra::clock::now_ms;
use crate::infra::repositories::databases::REPLICATION_USER;

/// An unbounded Postgres on a node with one core is the node, so the
/// form's empty answer is the form's default answer rather than "no
/// ceiling".
pub const DEFAULT_PRESET: u64 = 256 * 1024 * 1024;

pub struct CreateDatabase<'a> {
    pub project_id: &'a str,
    pub name: &'a str,
    pub version: &'a str,
    pub memory_limit: u64,
}

impl CreateDatabase<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> PlatformResult<(Service, Database)> {
        if !postgres::VERSIONS.contains(&self.version) {
            return Err(PlatformError::Refused(format!(
                "this node does not offer PostgreSQL {}",
                self.version
            )));
        }
        if !presets::LADDER.contains(&self.memory_limit) {
            return Err(PlatformError::Refused(
                "that is not one of the sizes on offer".into(),
            ));
        }
        if !postgres::initialises_in(self.version, self.memory_limit) {
            return Err(PlatformError::Refused(format!(
                "PostgreSQL {} cannot be initialised in {}: before 16 its initdb takes no \
                 setting for its 128 MB of shared buffers — choose 128 MB, or a newer version",
                self.version,
                presets::label(self.memory_limit)
            )));
        }

        let service = crate::infra::repositories::services::create(
            database,
            self.project_id,
            self.name,
            &postgres::image_for(self.version),
            &[],
        )
        .await?;

        // Marked before anything else reads it. A service that was a plain
        // container for even a moment is one the deploy path would have
        // started without its volume — which for a database is an empty
        // one that looks like it worked.
        crate::infra::repositories::services::set_kind(database, &service.id, Kind::Postgres)
            .await?;
        crate::infra::repositories::services::set_memory_limit(
            database,
            &service.id,
            Some(self.memory_limit),
        )
        .await?;

        // The volume is what makes it a database rather than a container
        // that forgets. Nothing else on the node declares one yet.
        crate::infra::repositories::volumes::create(
            database,
            &service.id,
            postgres::VOLUME,
            postgres::DATA_MOUNT,
        )
        .await?;

        // Declared and not published: the node has to know what the
        // container listens on to bind anything for it — an overlay port
        // for a standby elsewhere, or a host port if somebody publishes
        // it — and publishing is a separate decision somebody confirms.
        crate::infra::repositories::ports::create(
            database,
            &service.id,
            postgres::PORT,
            false,
            None,
            &[],
        )
        .await?;

        let identifier = identifier(&service.slug);
        let row = Database {
            service_id: service.id.clone(),
            engine: Engine::Postgres,
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
        };

        let stored = row.clone();
        database
            .write(move |connection| {
                connection.execute(
                    "INSERT INTO database \
                       (\"service_id\", \"engine\", \"version\", \"admin_user\", \
                        \"admin_password\", \"database_name\", \"replication_user\", \
                        \"replication_password\", \"primary_slot\", \"created_at\") \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                    ),
                )?;
                Ok(())
            })
            .await?;

        // The address the connection string names. Allocated now rather
        // than at the first deployment, because the page shows it before
        // anything has started.
        for replica in
            crate::infra::repositories::replicas::of_service(database, &service.id).await?
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
            version = self.version,
            limit = presets::label(self.memory_limit),
            "created a database"
        );
        Ok((service, row))
    }
}
