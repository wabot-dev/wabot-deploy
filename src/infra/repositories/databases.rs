//! Every `SELECT` and `INSERT` a [`Database`] needs.

use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::database::{Database, Engine};
use crate::domain::errors::PlatformResult;
use crate::infra::clock::now_ms;

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
    told: &crate::infra::network::errand::Database,
) -> PlatformResult<()> {
    let holds_the_primary = told.slots.contains(&told.primary_slot);
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
        (!holds_the_primary && !told.primary.is_empty()).then(|| told.primary.clone()),
        told.qualified_domain.clone(),
        holds_the_primary
            .then(|| serde_json::to_string(&told.replication_from).unwrap_or_default()),
        told.name.clone(),
        holds_the_primary && told.published,
    );
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO database \
                   (\"service_id\", \"engine\", \"version\", \"admin_user\", \
                    \"admin_password\", \"database_name\", \"replication_user\", \
                    \"replication_password\", \"primary_slot\", \"primary_endpoint\", \
                    \"owner_domain\", \"replication_from\", \"owner_name\", \"owner_published\", \
                    \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
                 ON CONFLICT(\"service_id\") DO UPDATE SET \
                   \"engine\" = ?2, \"version\" = ?3, \"admin_user\" = ?4, \
                   \"admin_password\" = ?5, \"database_name\" = ?6, \
                   \"replication_user\" = ?7, \"replication_password\" = ?8, \
                   \"primary_slot\" = ?9, \"primary_endpoint\" = ?10, \
                   \"owner_domain\" = ?11, \"replication_from\" = ?12, \
                   \"owner_name\" = ?13, \"owner_published\" = ?14",
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
                    row.11,
                    row.12,
                    row.13,
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
/// The node holding this database's primary, when it is not this one.
pub async fn primary_elsewhere(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Option<crate::infra::network::Node>> {
    let Some(row) = of_service(database, service_id).await? else {
        return Ok(None);
    };
    let Some(node_id) =
        crate::infra::repositories::replicas::in_slot(database, service_id, row.primary_slot)
            .await?
            .and_then(|replica| replica.node_id)
    else {
        return Ok(None);
    };
    crate::infra::network::find(database, &node_id)
        .await
        .map_err(|error| crate::domain::errors::PlatformError::Refused(error.to_string()))
}

/// When this node first held the database's row.
pub async fn created_at(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Option<i64>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"created_at\" FROM database WHERE \"service_id\" = ?1",
                    [service_id],
                    |row| row.get(0),
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
        replication_from: row
            .get::<_, Option<String>>(13)?
            .and_then(|text| serde_json::from_str(&text).ok()),
        owner_name: row.get(14)?,
        owner_published: row.get(15)?,
    })
}

const COLUMNS: &str = "\"service_id\", \"engine\", \"version\", \"admin_user\", \
                       \"admin_password\", \"database_name\", \"replication_user\", \
                       \"replication_password\", \"primary_slot\", \"primary_endpoint\", \
                       \"owner_domain\", \"restored_from\", \"recovery_target\", \
                       \"replication_from\", \"owner_name\", \"owner_published\"";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::service::Kind;
    use crate::domain::policies::postgres;
    use crate::features::databases::create_database_command::CreateDatabase;
    use crate::infra::deploy::standbys::dispatch;
    use crate::infra::repositories::ports;
    use crate::infra::repositories::replicas;
    use crate::infra::repositories::services;
    use crate::infra::repositories::volumes;

    async fn project() -> (SqliteDatabase, String) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "demo")
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
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 128 * 1024 * 1024,
            image: None,
            on: None,
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

    #[tokio::test]
    async fn a_mysql_database_is_made_with_its_own_image_volume_and_port() {
        let (database, project_id) = project().await;
        let (service, row) = CreateDatabase {
            project_id: &project_id,
            name: "Orders",
            engine: Engine::Mysql,
            version: "8.4",
            memory_limit: 256 * 1024 * 1024,
            image: None,
            on: None,
        }
        .run(&database)
        .await
        .expect("created");

        assert_eq!(service.kind, Kind::Mysql);
        assert_eq!(service.image, "docker.io/library/mysql:8.4");
        let volumes = volumes::of_service(&database, &service.id)
            .await
            .expect("volumes");
        assert_eq!(volumes[0].path, crate::domain::policies::mysql::DATA_MOUNT);
        let ports = ports::of_service(&database, &service.id)
            .await
            .expect("ports");
        assert_eq!(ports[0].container_port, 3306);

        let stored = of_service(&database, &service.id)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(stored, row);
        assert_eq!(stored.engine, Engine::Mysql);
        assert_eq!(stored.admin_user, "orders");
    }

    #[tokio::test]
    async fn each_engine_refuses_the_sizes_it_does_not_start_in() {
        let mb = 1024 * 1024;
        for (engine, version, refused, taken) in [
            (Engine::Postgres, "17", 64 * mb, 128 * mb),
            (Engine::Mysql, "8.4", 128 * mb, 256 * mb),
        ] {
            let (database, project_id) = project().await;
            let ask = |memory_limit| CreateDatabase {
                project_id: &project_id,
                name: "orders",
                engine,
                version,
                memory_limit,
                image: None,
                on: None,
            };
            let error = ask(refused).run(&database).await.expect_err("refused");
            assert!(error.to_string().contains("or more"), "{error}");
            assert!(services::all(&database, Some(&project_id))
                .await
                .expect("services")
                .is_empty());
            ask(taken).run(&database).await.expect("taken");
        }
    }

    #[tokio::test]
    async fn an_engine_this_node_does_not_offer_is_refused() {
        let (database, project_id) = project().await;
        let error = CreateDatabase {
            project_id: &project_id,
            name: "orders",
            engine: Engine::Unknown,
            version: "17",
            memory_limit: 256 * 1024 * 1024,
            image: None,
            on: None,
        }
        .run(&database)
        .await
        .expect_err("refused");
        assert!(error.to_string().contains("engine"), "{error}");
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
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 128 * 1024 * 1024,
            image: None,
            on: None,
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
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "16",
            memory_limit: 256 * 1024 * 1024,
            image: None,
            on: None,
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
        crate::infra::node::settings::set_domain(&database, Some("node.example"))
            .await
            .expect("domain");
        crate::infra::network::ensure_self(&database, &crate::infra::config::Config::default())
            .await
            .expect("self");
        let mut me = crate::infra::network::me(&database)
            .await
            .expect("query")
            .expect("there");
        me.overlay_ip = Some("10.42.0.1".into());
        crate::infra::network::save(&database, &me)
            .await
            .expect("saved");

        let (service, _) = CreateDatabase {
            project_id: &project_id,
            name: "orders",
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 256 * 1024 * 1024,
            image: None,
            on: None,
        }
        .run(&database)
        .await
        .expect("created");
        let here = crate::infra::repositories::replicas::of_service(&database, &service.id)
            .await
            .expect("replicas")
            .pop()
            .expect("the primary");
        crate::infra::repositories::replicas::ensure_overlay_port(&database, &here.id)
            .await
            .expect("port");

        crate::infra::network::save(
            &database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
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

        let told = crate::infra::network::errand::waiting(&database, "nd-far")
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

    async fn a_node(database: &SqliteDatabase, id: &str, overlay: &str) {
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
                overlay_ip: Some(overlay.into()),
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

    /// A primary made on another node is that node's to start, and it
    /// needs no address to do it — so it is told at once. Its standbys
    /// need its address, which only exists once it has deployed there and
    /// said so, so they are told nothing until then.
    ///
    /// Before this the dispatch returned at the first line for any
    /// database whose primary was not here, and `nubo-testing-db` read
    /// "Waiting for that node" for ever: nothing had been sent to it.
    #[tokio::test]
    async fn a_primary_elsewhere_is_told_at_once_and_its_standbys_once_it_answers() {
        let (database, project_id) = project().await;
        crate::infra::node::settings::set_domain(&database, Some("node.example"))
            .await
            .expect("domain");
        crate::infra::network::ensure_self(&database, &crate::infra::config::Config::default())
            .await
            .expect("self");
        let mut me = crate::infra::network::me(&database)
            .await
            .expect("query")
            .expect("there");
        me.overlay_ip = Some("10.42.0.1".into());
        crate::infra::network::save(&database, &me)
            .await
            .expect("saved");
        a_node(&database, "nd-far", "10.42.0.9").await;
        a_node(&database, "nd-third", "10.42.0.7").await;

        let (service, _) = CreateDatabase {
            project_id: &project_id,
            name: "orders",
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 256 * 1024 * 1024,
            image: None,
            on: Some("nd-far"),
        }
        .run(&database)
        .await
        .expect("created");
        replicas::ensure_count(&database, &service.id, 2, Some("nd-third"))
            .await
            .expect("a standby on a third node");
        replicas::ensure_count(&database, &service.id, 3, None)
            .await
            .expect("a standby here");
        let port = ports::of_service(&database, &service.id)
            .await
            .expect("ports")
            .pop()
            .expect("its port");
        ports::set_hostname(&database, &port.id, Some("db.shop.example"))
            .await
            .expect("named");
        ports::set_published(&database, &port.id, true)
            .await
            .expect("published");

        dispatch(
            &database,
            &service,
            None,
            Some("node.example".into()),
            &Default::default(),
        )
        .await
        .expect("dispatched");

        let errands = crate::infra::network::errand::waiting(&database, "nd-far")
            .await
            .expect("errands");
        let kinds: Vec<_> = errands.iter().map(|errand| errand.kind).collect();
        assert_eq!(
            kinds,
            [
                crate::infra::network::errand::Kind::Database,
                crate::infra::network::errand::Kind::Certificate
            ],
            "the database before its certificate, which is refused for a database not held"
        );
        let told = &errands[0];
        assert_eq!(told.payload["slots"], serde_json::json!([1]));
        assert_eq!(told.payload["primary"], serde_json::json!(""));
        assert_eq!(
            told.payload["replication_from"],
            serde_json::json!(["10.42.0.1/32", "10.42.0.7/32"]),
            "both standbys have to be admitted by a primary that knows neither"
        );
        assert_eq!(
            told.payload["name"],
            serde_json::json!("db.shop.example"),
            "a chosen name lives on a port row here, which never travels"
        );
        assert_eq!(told.payload["published"], serde_json::json!(true));
        let certificate = &errands[1].payload;
        assert_eq!(certificate["name"], serde_json::json!("db.shop.example"));
        assert_eq!(certificate["database"], serde_json::json!(true));
        let signed = crate::infra::edge::certs::load(&database, "db.shop.example")
            .await
            .expect("query")
            .expect("signed here");
        assert_eq!(
            certificate["cert_pem"],
            serde_json::json!(signed.cert_pem),
            "what the page here shows is what the primary there presents"
        );
        assert!(
            crate::infra::network::errand::waiting(&database, "nd-third")
                .await
                .expect("errands")
                .is_empty(),
            "a standby told to follow nowhere comes up as a primary of its own"
        );

        dispatch(
            &database,
            &service,
            Some(("10.42.0.9".into(), 30005)),
            Some("node.example".into()),
            &Default::default(),
        )
        .await
        .expect("dispatched again");

        let told = crate::infra::network::errand::waiting(&database, "nd-third")
            .await
            .expect("errands")
            .pop()
            .expect("the standby was told once there was somewhere to follow");
        assert_eq!(told.payload["slots"], serde_json::json!([2]));
        assert_eq!(
            told.payload["primary"],
            serde_json::json!("10.42.0.9:30005")
        );
        assert!(told.payload.get("replication_from").is_none());
        assert!(
            told.payload.get("published").is_none(),
            "only the primary answers on the published port"
        );
    }

    #[tokio::test]
    async fn a_database_is_made_only_on_a_node_that_agreed_to_keep_data() {
        let (database, project_id) = project().await;
        a_node(&database, "nd-far", "10.42.0.9").await;
        let mut host_only = crate::infra::network::find(&database, "nd-far")
            .await
            .expect("query")
            .expect("there");
        host_only.id = "nd-host".into();
        host_only.overlay_ip = Some("10.42.0.8".into());
        host_only.allows = vec![crate::domain::values::capability::Capability::Host];
        crate::infra::network::save(&database, &host_only)
            .await
            .expect("node");
        let ask = |name: &'static str, on: &'static str| CreateDatabase {
            project_id: &project_id,
            name,
            engine: crate::domain::entities::database::Engine::Mysql,
            version: "8.4",
            memory_limit: 256 * 1024 * 1024,
            image: None,
            on: Some(on),
        };

        let error = ask("refused", "nd-host")
            .run(&database)
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("keep data"), "{error}");
        assert!(services::all(&database, Some(&project_id))
            .await
            .expect("services")
            .is_empty());

        let (made, _) = ask("orders", "nd-far").run(&database).await.expect("made");
        let first = replicas::of_service(&database, &made.id)
            .await
            .expect("replicas")
            .pop()
            .expect("its first copy");
        assert_eq!(first.node_id.as_deref(), Some("nd-far"));
        assert_eq!(
            first.reserved_host, None,
            "an address in this node's bridge names nothing on the node running it"
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
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 256 * 1024 * 1024,
            image: None,
            on: None,
        }
        .run(&database)
        .await
        .expect("created");

        let mut told = crate::infra::network::errand::Database {
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
            replication_from: Vec::new(),
            qualified_domain: Some("owner.example".into()),
            name: None,
            published: false,
            running: true,
        };
        adopt(&database, &service.id, &told).await.expect("adopt");
        assert_eq!(
            of_service(&database, &service.id)
                .await
                .expect("query")
                .expect("there")
                .replication_from,
            None,
            "a node holding only standbys derives nothing from a list"
        );

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

        // And then asked to hold the primary as well: it follows nobody,
        // and admits the standbys it was told about.
        told.slots = vec![1];
        told.primary = String::new();
        told.replication_from = vec!["10.42.0.1/32".into(), "10.42.0.3/32".into()];
        adopt(&database, &service.id, &told)
            .await
            .expect("the primary");

        let found = of_service(&database, &service.id)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(found.primary_endpoint, None);
        assert_eq!(
            found.replication_from,
            Some(vec!["10.42.0.1/32".into(), "10.42.0.3/32".into()])
        );
        assert_eq!(found.role_of(1), postgres::Role::Primary);
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
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "9.6",
            memory_limit: 128 * 1024 * 1024,
            image: None,
            on: None,
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
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 100,
            image: None,
            on: None,
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
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 128 * 1024 * 1024,
            image: None,
            on: None,
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

    #[test]
    fn an_engine_reads_back_as_what_it_was_written_as() {
        for engine in Engine::OFFERED {
            assert_eq!(Engine::parse(engine.as_str()), engine);
            assert_eq!(engine.kind().engine(), Some(engine));
        }
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
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 128 * 1024 * 1024,
            image: None,
            on: None,
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
