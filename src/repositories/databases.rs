//! Every `SELECT` and `INSERT` a [`Database`] needs.

use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::SqliteDatabase;

use crate::entities::database::{Database, Engine};
use crate::platform::{now_ms, PlatformResult};

pub const REPLICATION_USER: &str = "wabot_replication";

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
        restored_from: row.get(11)?,
        recovery_target: row.get(12)?,
    })
}

const COLUMNS: &str = "\"service_id\", \"engine\", \"version\", \"admin_user\", \
                       \"admin_password\", \"database_name\", \"replication_user\", \
                       \"replication_password\", \"primary_slot\", \"primary_endpoint\", \
                       \"owner_domain\", \"restored_from\", \"recovery_target\"";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::create_database::CreateDatabase;
    use crate::commands::dispatch_standbys::dispatch;
    use crate::entities::service::Kind;
    use crate::policies::postgres;
    use crate::repositories::ports;
    use crate::repositories::replicas;
    use crate::repositories::services;
    use crate::repositories::volumes;

    async fn project() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = crate::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        (database, project.id)
    }

    /// Everything a database is, written in one operation. Each of
    /// these is a thing the deploy path reads, and a database missing
    /// any one of them starts as something that looks like it worked.
    #[tokio::test]
    async fn creating_one_writes_the_service_the_volume_and_the_port() {
        let (database, project_id) = project().await;
        let (service, row) = CreateDatabase {
            project_id: &project_id,
            name: "Orders",
            version: "17",
            memory_limit: 128 * 1024 * 1024,
        }
        .run(&database)
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
        let (service, _) = CreateDatabase {
            project_id: &project_id,
            name: "orders",
            version: "17",
            memory_limit: 64 * 1024 * 1024,
        }
        .run(&database)
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
        let (service, made) = CreateDatabase {
            project_id: &project_id,
            name: "orders",
            version: "16",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&database)
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
    /// A node that has just lost its last copy is the one that most needs
    /// telling, and it is the one the rows no longer point at.
    ///
    /// Found on the test node: a database reduced from three copies to one
    /// left a Postgres running on the other machine, holding a volume and
    /// following a primary, with nothing on either side claiming it. The
    /// `host` path had solved this with the same set of "nodes touched
    /// before the change"; this one had the same hole and no `slots: []`
    /// ever left.
    #[tokio::test]
    async fn a_node_that_lost_its_last_copy_is_told_to_let_go() {
        let (database, project_id) = project().await;
        crate::node::settings::set_domain(&database, Some("node.example"))
            .await
            .expect("domain");
        crate::network::ensure_self(&database, &crate::config::Config::default())
            .await
            .expect("self");
        let mut me = crate::network::me(&database)
            .await
            .expect("query")
            .expect("there");
        me.overlay_ip = Some("10.42.0.1".into());
        crate::network::save(&database, &me).await.expect("saved");

        let (service, _) = CreateDatabase {
            project_id: &project_id,
            name: "orders",
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&database)
        .await
        .expect("created");
        let here = crate::repositories::replicas::of_service(&database, &service.id)
            .await
            .expect("replicas")
            .pop()
            .expect("the primary");
        crate::repositories::replicas::ensure_overlay_port(&database, &here.id)
            .await
            .expect("port");

        crate::network::save(
            &database,
            &crate::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-far".into(),
                name: "far.example".into(),
                kind: crate::network::Kind::Private,
                overlay_ip: Some("10.42.0.9".into()),
                public_key: None,
                endpoint: None,
                allows: vec![crate::network::capability::Capability::Store],
                last_seen_at: None,
                is_self: false,
                ca_pem: None,
            },
        )
        .await
        .expect("node");

        // The copy is gone from the rows — which is the state a placement
        // change leaves behind, and the reason nothing points at that node
        // any more.
        let touched: std::collections::BTreeSet<String> =
            ["nd-far".to_string()].into_iter().collect();
        dispatch(
            &database,
            &service,
            Some(("10.42.0.1".into(), 30000)),
            Some("node.example".into()),
            &touched,
        )
        .await
        .expect("dispatched");

        let told = crate::network::errand::waiting(&database, "nd-far")
            .await
            .expect("errands")
            .pop()
            .expect("the node holding nothing was told nothing");
        assert_eq!(
            told.payload["slots"],
            serde_json::json!([]),
            "which is how it learns to stop and take the copy off its disk"
        );
    }

    /// This guards a bug that reached both nodes: `qualified_domain`
    /// was read off the errand into the parameter tuple and then left
    /// out of the column list, so the copy went on naming itself under
    /// the *holding* node's domain and `psql` refused its certificate.
    /// Nothing complained — a tuple element nobody binds is not a
    /// warning.
    #[tokio::test]
    async fn an_adopted_row_keeps_everything_the_errand_carried() {
        let (database, project_id) = project().await;
        let (service, _) = CreateDatabase {
            project_id: &project_id,
            name: "orders",
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&database)
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
        let error = CreateDatabase {
            project_id: &project_id,
            name: "orders",
            version: "9.6",
            memory_limit: 64 * 1024 * 1024,
        }
        .run(&database)
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
        assert!(CreateDatabase {
            project_id: &project_id,
            name: "orders",
            version: "17",
            memory_limit: 100,
        }
        .run(&database)
        .await
        .is_err());
    }

    /// Slot 1 accepts writes and everything else follows it. Read
    /// through `primary_slot` rather than compared to 1, so promoting
    /// one later is a row.
    #[tokio::test]
    async fn the_primary_slot_decides_which_copy_takes_writes() {
        let (database, project_id) = project().await;
        let (_, row) = CreateDatabase {
            project_id: &project_id,
            name: "orders",
            version: "17",
            memory_limit: 64 * 1024 * 1024,
        }
        .run(&database)
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
        let (service, _) = CreateDatabase {
            project_id: &project_id,
            name: "orders",
            version: "17",
            memory_limit: 64 * 1024 * 1024,
        }
        .run(&database)
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
