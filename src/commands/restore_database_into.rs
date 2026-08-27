//! Unpack one database's volume beside the original, which goes on
//! serving.

use wabot::sqlite::SqliteDatabase;

use crate::clock::now_ms;
use crate::entities::database::Database;
use crate::entities::service::{Kind, Service};
use crate::errors::PlatformResult;
use crate::policies::postgres;

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
/// * It is marked as recovering, so the deploy path unpacks and replays
///   rather than running `initdb` into an empty volume — which would
///   produce an empty database that looks exactly like a successful
///   restore.
pub async fn restore_into(
    database: &SqliteDatabase,
    source: &Database,
    project_id: &str,
    name: &str,
    memory_limit: u64,
    // The backup directory it unpacks, and the moment it stops at.
    from: &str,
    target: Option<&str>,
) -> PlatformResult<(Service, Database)> {
    let service = crate::commands::create_service::CreateService {
        project_id,
        name,
        image: &postgres::image_for(&source.version),
        env: &[],
    }
    .run(database)
    .await?;
    crate::repositories::services::set_kind(database, &service.id, Kind::Postgres).await?;
    crate::repositories::services::set_memory_limit(database, &service.id, Some(memory_limit))
        .await?;
    crate::repositories::volumes::create(
        database,
        &service.id,
        postgres::VOLUME,
        postgres::DATA_MOUNT,
    )
    .await?;
    crate::repositories::ports::create(database, &service.id, postgres::PORT, false, None, &[])
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
