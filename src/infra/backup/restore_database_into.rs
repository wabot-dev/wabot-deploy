//! Unpack one database's volume beside the original, which goes on
//! serving.

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::database::Database;
use crate::domain::entities::service::Service;
use crate::domain::errors::PlatformResult;
use crate::domain::policies::postgres;
use crate::infra::clock::now_ms;

/// Make a database that will be restored into, rather than initialised.
///
/// **A new database, never the original rewound.** Rewinding is
/// irreversible — everything after the chosen moment is gone, which is
/// exactly what somebody looking for one dropped table does not want —
/// and it breaks the standbys, which are then ahead of their primary and
/// have to be seeded again. A copy leaves the original untouched: take
/// what you came for and throw the copy away.
///
/// Almost `create`, and the differences are the point:
///
/// * The **credentials are the original's**. A restored copy that
///   generated new ones would be a database nobody's connection string
///   opens, at the moment somebody is in a hurry.
/// * The **version is the original's**, because a data directory built
///   by 17 is not one 18 will open.
/// * So is the **image**, and it is not derived from the version: a
///   database made with its own image has files written by that image's
///   user under that image's C library, and the node's default for the
///   same version is a different user and a different collation.
/// * It is marked as recovering, so the deploy path unpacks and replays
///   rather than running `initdb` into an empty volume — which would
///   produce an empty database that looks exactly like a successful
///   restore.
pub async fn restore_into(
    database: &SqliteDatabase,
    source: &Database,
    original: &Service,
    project_id: &str,
    name: &str,
    // The backup directory it unpacks, and the moment it stops at.
    from: &str,
    target: Option<&str>,
) -> PlatformResult<(Service, Database)> {
    let memory_limit = original
        .memory_limit
        .unwrap_or(crate::domain::policies::presets::SMALLEST);
    let service = crate::infra::repositories::services::create(
        database,
        project_id,
        name,
        &original.image,
        &[],
    )
    .await?;
    crate::infra::repositories::services::set_kind(database, &service.id, source.engine.kind())
        .await?;
    crate::infra::repositories::services::set_memory_limit(
        database,
        &service.id,
        Some(memory_limit),
    )
    .await?;
    crate::infra::repositories::volumes::create(
        database,
        &service.id,
        postgres::VOLUME,
        source.engine.data_mount(),
    )
    .await?;
    crate::infra::repositories::ports::create(
        database,
        &service.id,
        source.engine.port(),
        false,
        None,
        &[],
    )
    .await?;

    let row = Database {
        service_id: service.id.clone(),
        engine: source.engine,
        version: source.version.clone(),
        // The original's, so the string somebody already has opens it.
        admin_user: source.admin_user.clone(),
        admin_password: source.admin_password.clone(),
        database_name: source.database_name.clone(),
        replication_user: source.replication_user.clone(),
        replication_password: source.replication_password.clone(),
        primary_slot: 1,
        primary_endpoint: None,
        owner_domain: None,
        restored_from: Some(from.to_string()),
        recovery_target: target.map(str::to_string),
    };

    let stored = row.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO database \
                   (\"service_id\", \"engine\", \"version\", \"admin_user\", \
                    \"admin_password\", \"database_name\", \"replication_user\", \
                    \"replication_password\", \"primary_slot\", \"restored_from\", \
                    \"recovery_target\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                    stored.restored_from,
                    stored.recovery_target,
                    now_ms(),
                ),
            )?;
            Ok(())
        })
        .await?;

    Ok((service, row))
}

#[cfg(test)]
mod tests {
    use crate::domain::entities::database::Engine;
    use crate::features::databases::create_database_command::CreateDatabase;

    /// The copy used to take the node's default for the version, so a
    /// database made from `pgvector/pgvector` — Debian, its user 999 —
    /// would have been unpacked by and started under Alpine's, user 70,
    /// with another C library's idea of how text sorts.
    #[tokio::test]
    async fn a_restored_copy_runs_the_image_the_original_was_made_with() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        let (original, row) = CreateDatabase {
            project_id: &project.id,
            name: "Vectors",
            engine: Engine::Postgres,
            version: "17",
            memory_limit: 256 * 1024 * 1024,
            image: Some("docker.io/pgvector/pgvector:pg17-trixie"),
        }
        .run(&database)
        .await
        .expect("created");

        let (copy, copied) = super::restore_into(
            &database,
            &row,
            &original,
            &project.id,
            "vectors-restored",
            "/var/lib/wabot-deploy/backups/2026-11-01/volumes/abc",
            None,
        )
        .await
        .expect("restored");

        assert_eq!(copy.image, "docker.io/pgvector/pgvector:pg17-trixie");
        assert_eq!(copied.version, "17");
        let stored = crate::infra::repositories::services::find(&database, &copy.id)
            .await
            .expect("read")
            .expect("there");
        assert_eq!(stored.memory_limit, original.memory_limit);
    }
}
