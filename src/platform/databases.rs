//! A managed database: the service, and the engine's own row.
//!
//! ## What creating one writes
//!
//! A service, so that deploying, reconciling, observing, placing a copy
//! elsewhere, reporting and eviction are the ones that already work; a
//! volume, so the data outlives the container; a port, so the node
//! knows what to bind; and this row, which holds what only an engine
//! has — a version, credentials, and which copy accepts writes.
//!
//! ## The credentials are not in the environment
//!
//! They are derived into it at deploy time from this row. A service's
//! `env` is editable, arrives on errands and is shown on a page; the
//! password a database was created with is none of those things, and
//! two copies of it are one copy that can be wrong.
//!
//! ## Slot 1 accepts writes
//!
//! Through `primary_slot`, which exists so that promoting a standby is
//! a row rather than a migration. Nothing writes it — see
//! `docs/databases.md`.

use serde::Serialize;
use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::SqliteDatabase;

use super::services::Service;
use super::{now_ms, postgres, presets, PlatformError, PlatformResult};

/// Which engine. One today, and the enum is here rather than a string
/// for the same reason `errand::Kind`'s is: the row can outlive the
/// version that understands it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Postgres,
    /// A row written by a newer node. Held and refused, never parsed
    /// into a panic.
    Unknown,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Postgres => "postgres",
            Engine::Unknown => "unknown",
        }
    }

    /// What the console calls it. Not translated — it is a product
    /// name.
    pub fn label(self) -> &'static str {
        match self {
            Engine::Postgres => "PostgreSQL",
            Engine::Unknown => "unknown",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text {
            "postgres" => Engine::Postgres,
            _ => Engine::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Database {
    pub service_id: String,
    pub engine: Engine,
    /// The major version, as the image tags spell it.
    pub version: String,
    pub admin_user: String,
    pub admin_password: String,
    pub database_name: String,
    pub replication_user: String,
    pub replication_password: String,
    /// Which slot accepts writes.
    pub primary_slot: u32,
    /// Where the primary answers, when this node was **told** rather
    /// than being able to work it out.
    ///
    /// `None` on the node that owns the database: it derives the
    /// address from its own rows, and a stored copy would be a second
    /// answer able to go stale. `Some` on a node holding a standby,
    /// which has no row for the primary and never will.
    pub primary_endpoint: Option<String>,
    /// Whose domain the qualified name is built from.
    ///
    /// `None` is this node's own. `Some` on a copy held for somebody
    /// else — see migration `0032`: a name that changed with the
    /// machine holding it was no name at all.
    pub owner_domain: Option<String>,
}

impl Database {
    /// Whether this copy is the one that accepts writes.
    pub fn role_of(&self, slot: u32) -> postgres::Role {
        match slot == self.primary_slot {
            true => postgres::Role::Primary,
            false => postgres::Role::Standby,
        }
    }
}

/// Make a database: the service, its volume, its port and this row.
///
/// `version` is a major from [`postgres::VERSIONS`] and `memory_limit`
/// a rung from [`presets::LADDER`]. Both are refused rather than
/// rounded: the first would be a pull that fails minutes later against
/// a tag Docker Hub does not have, and the second a page showing a
/// number the container does not have.
pub async fn create(
    database: &SqliteDatabase,
    project_id: &str,
    name: &str,
    version: &str,
    memory_limit: u64,
) -> PlatformResult<(Service, Database)> {
    if !postgres::VERSIONS.contains(&version) {
        return Err(PlatformError::Refused(format!(
            "this node does not offer PostgreSQL {version}"
        )));
    }
    if !presets::LADDER.contains(&memory_limit) {
        return Err(PlatformError::Refused(
            "that is not one of the sizes on offer".into(),
        ));
    }

    let service = super::services::create(
        database,
        project_id,
        name,
        &postgres::image_for(version),
        &[],
    )
    .await?;

    // Marked before anything else reads it. A service that was a plain
    // container for even a moment is one the deploy path would have
    // started without its volume — which for a database is an empty
    // one that looks like it worked.
    super::services::set_kind(database, &service.id, Kind::Postgres).await?;
    super::services::set_memory_limit(database, &service.id, Some(memory_limit)).await?;

    // The volume is what makes it a database rather than a container
    // that forgets. Nothing else on the node declares one yet.
    super::volumes::create(
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
    super::ports::create(database, &service.id, postgres::PORT, false, None).await?;

    let identifier = identifier(&service.slug);
    let row = Database {
        service_id: service.id.clone(),
        engine: Engine::Postgres,
        version: version.to_string(),
        admin_user: identifier.clone(),
        admin_password: wabot::prelude::password::generate(24),
        database_name: identifier,
        // A fixed name, because it is this node's own role rather than
        // anything the operator chose, and one less thing that can
        // differ between the primary's `pg_hba.conf` and a standby's
        // conninfo.
        replication_user: REPLICATION_USER.to_string(),
        replication_password: wabot::prelude::password::generate(24),
        primary_slot: 1,
        // Derived here, always: this node owns the database, so it can
        // read where its own primary answers out of its own rows, and
        // name it under its own domain.
        primary_endpoint: None,
        owner_domain: None,
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
    for replica in super::replicas::of_service(database, &service.id).await? {
        super::replicas::reserve_host(database, project_id, &replica.id).await?;
    }

    // Read back: the row carries the kind and the ceiling that were
    // set after it was made, and a caller handed the `create` copy
    // would see neither.
    let service = super::services::find(database, &service.id)
        .await?
        .ok_or_else(|| PlatformError::Refused("the service vanished as it was made".into()))?;

    tracing::info!(
        service = %service.slug,
        version,
        limit = presets::label(memory_limit),
        "created a database"
    );
    Ok((service, row))
}

use super::services::Kind;

/// The role every managed Postgres has from birth.
pub const REPLICATION_USER: &str = "wabot_replication";

/// A slug as a Postgres identifier.
///
/// Hyphens become underscores and a leading digit gets a prefix,
/// because an identifier that needs quoting is one every client has to
/// remember to quote — and the one place it will be forgotten is a
/// connection string somebody types by hand.
fn identifier(slug: &str) -> String {
    let mut identifier: String = slug
        .chars()
        .map(|c| match c.is_ascii_alphanumeric() {
            true => c.to_ascii_lowercase(),
            false => '_',
        })
        .collect();

    if identifier.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        identifier.insert_str(0, "db_");
    }
    // Postgres truncates at 63 bytes and says nothing, so the name in
    // the connection string would stop matching the one it made.
    identifier.truncate(63);
    identifier
}

/// Tell every node holding a standby of this database what to hold.
///
/// **Recomputed, not emitted.** The port the primary answers on comes
/// out of this node's own port space and is assigned when the primary
/// deploys, so an errand written at the moment somebody clicked would
/// carry an address that did not exist yet. This is called after
/// anything that could have settled it, and `queue_if_changed` makes
/// calling it often free.
///
/// Nothing is queued while the primary has no address: a standby told to
/// follow nowhere would come up as a primary of its own, holding a copy
/// of somebody's data and accepting writes into it.
pub async fn dispatch(
    database: &SqliteDatabase,
    service: &Service,
    primary: Option<(String, u16)>,
    domain: Option<String>,
) -> PlatformResult<usize> {
    let Some(row) = of_service(database, &service.id).await? else {
        return Ok(0);
    };
    // A database this node was told to hold is not this node's to place
    // anywhere. Its own authority decides.
    if !service.is_ours() || row.primary_endpoint.is_some() {
        return Ok(0);
    }
    let Some((host, port)) = primary else {
        return Ok(0);
    };

    let project = super::projects::find(database, &service.project_id)
        .await?
        .ok_or_else(|| PlatformError::Refused("no project for this database".into()))?;
    let running = super::services::find(database, &service.id)
        .await?
        .map_or(service.desired_state, |row| row.desired_state);
    let placements = super::replicas::of_service(database, &service.id).await?;

    // Which node holds which slots. A node holding two copies is told
    // about both in one instruction, because the instruction is the
    // whole of what that node runs for this database — the same rule a
    // `host` errand follows, and what lets a copy be taken away.
    let mut by_node: std::collections::BTreeMap<String, Vec<u32>> = Default::default();
    for replica in placements.iter().filter(|r| !r.evicted()) {
        let Some(node_id) = &replica.node_id else {
            continue;
        };
        if replica.slot == row.primary_slot {
            continue;
        }
        by_node
            .entry(node_id.clone())
            .or_default()
            .push(replica.slot);
    }

    let store = crate::network::capability::Capability::Store;
    let nodes = crate::network::all(database)
        .await
        .map_err(|error| PlatformError::Refused(error.to_string()))?;
    let mut sent = 0;

    for (node_id, slots) in by_node {
        // A node that never agreed to keep data is not somewhere a copy
        // can go, and queueing against it produces an errand it refuses
        // for ever while this side says the copy is placed.
        if !nodes
            .iter()
            .any(|node| node.id == node_id && node.allows.contains(&store))
        {
            tracing::debug!(node = %node_id, "skipped: it has not agreed to keep data here");
            continue;
        }

        let payload = serde_json::to_value(crate::network::errand::Standby {
            project: project.name.clone(),
            service: service.name.clone(),
            image: service.image.clone(),
            registry: super::registry_credentials::host_of(&service.image).unwrap_or_default(),
            // No credential: a database's image comes from Docker Hub,
            // which serves anybody. `send_there` explains what happens
            // when one is sent to a registry that is not this node's.
            username: None,
            secret: None,
            memory_limit: service.memory_limit.unwrap_or(super::presets::SMALLEST),
            engine: row.engine.as_str().to_string(),
            version: row.version.clone(),
            database_name: row.database_name.clone(),
            admin_user: row.admin_user.clone(),
            admin_password: row.admin_password.clone(),
            replication_user: row.replication_user.clone(),
            replication_password: row.replication_password.clone(),
            primary: format!("{host}:{port}"),
            slots,
            primary_slot: row.primary_slot,
            // This node's, because this node owns the database — and
            // the copy has to answer to *its* name, not to one built
            // from the domain of whichever machine is holding it.
            qualified_domain: domain.clone(),
            // The intent travels with the instruction, and it is read
            // from the **row** rather than from the caller's `service`.
            // `stop` writes the state and then dispatches with the struct
            // it was handed, which still says `Running`: a stop that
            // travelled asked for a deployment.
            running: running == super::services::DesiredState::Running,
        })
        .map_err(|error| PlatformError::Refused(error.to_string()))?;

        let queued = crate::network::errand::queue_if_changed(
            database,
            &node_id,
            crate::network::errand::Kind::Database,
            // One database, so a node holding standbys of two is told
            // about each without one overwriting the other's history.
            &format!("database:{}", service.id),
            &payload,
        )
        .await
        .map_err(|error| PlatformError::Refused(error.to_string()))?;

        if queued.is_some() {
            tracing::info!(service = %service.slug, node = %node_id, "asked a node to hold a standby");
            sent += 1;
        }
    }
    Ok(sent)
}

/// Write the engine row for a database this node was *told* to hold.
///
/// The credentials are the sending node's, in full, because a copy with
/// different ones is not a copy: the seed logs in as the replication
/// role and clients log in as the ordinary one, and both have to be the
/// same as the primary's.
///
/// `primary_endpoint` is the difference from [`create`]. A node that
/// owns a database works out where its primary answers from its own
/// rows; this node has none of them and never will, so it keeps what it
/// was told. See migration `0031`.
///
/// Convergent: a second errand updates rather than refuses, because the
/// far end hands one over again whenever the answer did not get back.
pub async fn adopt(
    database: &SqliteDatabase,
    service_id: &str,
    told: &crate::network::errand::Standby,
) -> PlatformResult<()> {
    let row = (
        service_id.to_string(),
        Engine::parse(&told.engine).as_str().to_string(),
        told.version.clone(),
        told.admin_user.clone(),
        told.admin_password.clone(),
        told.database_name.clone(),
        told.replication_user.clone(),
        told.replication_password.clone(),
        i64::from(told.primary_slot),
        told.primary.clone(),
        told.qualified_domain.clone(),
    );
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO database \
                   (\"service_id\", \"engine\", \"version\", \"admin_user\", \
                    \"admin_password\", \"database_name\", \"replication_user\", \
                    \"replication_password\", \"primary_slot\", \"primary_endpoint\", \
                    \"owner_domain\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
                 ON CONFLICT(\"service_id\") DO UPDATE SET \
                   \"engine\" = ?2, \"version\" = ?3, \"admin_user\" = ?4, \
                   \"admin_password\" = ?5, \"database_name\" = ?6, \
                   \"replication_user\" = ?7, \"replication_password\" = ?8, \
                   \"primary_slot\" = ?9, \"primary_endpoint\" = ?10, \
                   \"owner_domain\" = ?11",
                (
                    row.0,
                    row.1,
                    row.2,
                    row.3,
                    row.4,
                    row.5,
                    row.6,
                    row.7,
                    row.8,
                    row.9,
                    row.10,
                    now_ms(),
                ),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Option<Database>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM database WHERE \"service_id\" = ?1"),
                    [service_id],
                    decode,
                )
                .optional()
        })
        .await?)
}

const COLUMNS: &str = "\"service_id\", \"engine\", \"version\", \"admin_user\", \
                       \"admin_password\", \"database_name\", \"replication_user\", \
                       \"replication_password\", \"primary_slot\", \"primary_endpoint\", \
                       \"owner_domain\"";

fn decode(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Database> {
    Ok(Database {
        service_id: row.get(0)?,
        engine: Engine::parse(&row.get::<_, String>(1)?),
        version: row.get(2)?,
        admin_user: row.get(3)?,
        admin_password: row.get(4)?,
        database_name: row.get(5)?,
        replication_user: row.get(6)?,
        replication_password: row.get(7)?,
        primary_slot: row.get::<_, i64>(8)? as u32,
        primary_endpoint: row.get(9)?,
        owner_domain: row.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{ports, projects, replicas, services, volumes};

    async fn project() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "demo").await.expect("project");
        (database, project.id)
    }

    /// Everything a database is, written in one operation. Each of
    /// these is a thing the deploy path reads, and a database missing
    /// any one of them starts as something that looks like it worked.
    #[tokio::test]
    async fn creating_one_writes_the_service_the_volume_and_the_port() {
        let (database, project_id) = project().await;
        let (service, row) = create(&database, &project_id, "Orders", "17", 128 * 1024 * 1024)
            .await
            .expect("created");

        assert_eq!(service.kind, Kind::Postgres);
        assert_eq!(service.image, "docker.io/library/postgres:17-alpine");
        assert_eq!(service.memory_limit, Some(128 * 1024 * 1024));
        assert!(
            service.env.is_empty(),
            "the credentials belong to the row, not to an editable environment"
        );

        let volumes = volumes::of_service(&database, &service.id)
            .await
            .expect("volumes");
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].path, postgres::DATA_MOUNT);

        let ports = ports::of_service(&database, &service.id)
            .await
            .expect("ports");
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].container_port, 5432);
        assert!(
            ports[0].host_port.is_none(),
            "publishing is its own decision"
        );
        assert!(
            ports[0].hostname.is_none(),
            "a database does not serve HTTPS"
        );

        assert_eq!(row.engine, Engine::Postgres);
        assert_eq!(row.database_name, "orders");
        assert_eq!(row.admin_user, "orders");
        assert_eq!(row.replication_user, REPLICATION_USER);
        assert_ne!(row.admin_password, row.replication_password);
        assert_eq!(row.primary_slot, 1);
    }

    /// The address a connection string names cannot move when the
    /// container is recreated, so it is allocated when the database is
    /// made rather than when it first starts.
    #[tokio::test]
    async fn the_first_copy_has_its_address_before_anything_runs() {
        let (database, project_id) = project().await;
        let (service, _) = create(&database, &project_id, "orders", "17", 64 * 1024 * 1024)
            .await
            .expect("created");

        let replicas = replicas::of_service(&database, &service.id)
            .await
            .expect("replicas");
        assert_eq!(replicas.len(), 1);
        assert!(
            replicas[0].reserved_host.is_some(),
            "the page would have no address to show"
        );
    }

    #[tokio::test]
    async fn the_row_reads_back() {
        let (database, project_id) = project().await;
        let (service, made) = create(&database, &project_id, "orders", "16", 256 * 1024 * 1024)
            .await
            .expect("created");

        let found = of_service(&database, &service.id)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(found, made);
        assert_eq!(found.version, "16");
    }

    /// Everything an errand carries reaches the row, and a second
    /// errand updates rather than being ignored.
    ///
    /// This guards a bug that reached both nodes: `qualified_domain`
    /// was read off the errand into the parameter tuple and then left
    /// out of the column list, so the copy went on naming itself under
    /// the *holding* node's domain and `psql` refused its certificate.
    /// Nothing complained — a tuple element nobody binds is not a
    /// warning.
    #[tokio::test]
    async fn an_adopted_row_keeps_everything_the_errand_carried() {
        let (database, project_id) = project().await;
        let (service, _) = create(&database, &project_id, "orders", "17", 256 * 1024 * 1024)
            .await
            .expect("created");

        let mut told = crate::network::errand::Standby {
            project: "db-test".into(),
            service: "orders".into(),
            image: "docker.io/library/postgres:17".into(),
            registry: "docker.io".into(),
            username: None,
            secret: None,
            memory_limit: 256 * 1024 * 1024,
            engine: "postgres".into(),
            version: "17".into(),
            database_name: "orders".into(),
            admin_user: "orders".into(),
            admin_password: "secret".into(),
            replication_user: "wabot_replication".into(),
            replication_password: "also-secret".into(),
            primary: "10.42.0.1:30002".into(),
            slots: vec![3],
            primary_slot: 1,
            qualified_domain: Some("owner.example".into()),
            running: true,
        };
        adopt(&database, &service.id, &told).await.expect("adopt");

        let found = of_service(&database, &service.id)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(found.owner_domain.as_deref(), Some("owner.example"));
        assert_eq!(found.primary_endpoint.as_deref(), Some("10.42.0.1:30002"));
        assert_eq!(found.replication_password, "also-secret");

        // The owner moved to a domain of its own, and says so again.
        told.qualified_domain = Some("elsewhere.example".into());
        told.primary = "10.42.0.1:30007".into();
        adopt(&database, &service.id, &told).await.expect("again");

        let found = of_service(&database, &service.id)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(found.owner_domain.as_deref(), Some("elsewhere.example"));
        assert_eq!(found.primary_endpoint.as_deref(), Some("10.42.0.1:30007"));
    }

    /// A version this node does not offer is a pull that fails minutes
    /// later against a tag Docker Hub does not have. Refusing in the
    /// form is the same answer, said where somebody can act on it.
    #[tokio::test]
    async fn a_version_that_is_not_offered_is_refused_before_anything_is_written() {
        let (database, project_id) = project().await;
        let error = create(&database, &project_id, "orders", "9.6", 64 * 1024 * 1024)
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("does not offer"), "{error}");

        assert!(
            services::all(&database, Some(&project_id))
                .await
                .expect("services")
                .is_empty(),
            "a service was left behind by a refusal"
        );
    }

    #[tokio::test]
    async fn a_size_that_is_not_on_the_ladder_is_refused() {
        let (database, project_id) = project().await;
        assert!(create(&database, &project_id, "orders", "17", 100)
            .await
            .is_err());
    }

    /// An identifier that needs quoting is one every client has to
    /// remember to quote, and the place it gets forgotten is a
    /// connection string somebody types.
    #[test]
    fn an_identifier_never_needs_quoting() {
        assert_eq!(identifier("orders"), "orders");
        assert_eq!(identifier("orders-api"), "orders_api");
        assert_eq!(identifier("2fa"), "db_2fa");
        assert_eq!(identifier(""), "db_");
        assert_eq!(identifier(&"a".repeat(100)).len(), 63);

        for slug in ["orders", "orders-api", "2fa", "x-1-y"] {
            let identifier = identifier(slug);
            assert!(
                identifier
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{identifier}"
            );
            assert!(!identifier.starts_with(|c: char| c.is_ascii_digit()));
        }
    }

    /// Slot 1 accepts writes and everything else follows it. Read
    /// through `primary_slot` rather than compared to 1, so promoting
    /// one later is a row.
    #[tokio::test]
    async fn the_primary_slot_decides_which_copy_takes_writes() {
        let (database, project_id) = project().await;
        let (_, row) = create(&database, &project_id, "orders", "17", 64 * 1024 * 1024)
            .await
            .expect("created");

        assert_eq!(row.role_of(1), postgres::Role::Primary);
        assert_eq!(row.role_of(2), postgres::Role::Standby);
        assert_eq!(row.role_of(7), postgres::Role::Standby);
    }

    /// A row written by a newer node is a value this one can hold and
    /// refuse, not a parse failure that takes a page down.
    #[test]
    fn an_engine_this_version_does_not_know_is_a_value() {
        assert_eq!(Engine::parse("postgres"), Engine::Postgres);
        assert_eq!(Engine::parse("mariadb"), Engine::Unknown);
        assert_eq!(Engine::parse(""), Engine::Unknown);
    }

    /// Deleting the service takes the engine's row, its volume rows and
    /// its ports. It does not take the bytes — that is `volumes::discard`,
    /// and its only caller is a confirmation somebody typed.
    #[tokio::test]
    async fn the_row_goes_with_the_service() {
        let (database, project_id) = project().await;
        let (service, _) = create(&database, &project_id, "orders", "17", 64 * 1024 * 1024)
            .await
            .expect("created");

        services::delete(&database, &service.id)
            .await
            .expect("delete");
        assert!(of_service(&database, &service.id)
            .await
            .expect("query")
            .is_none());
    }
}
