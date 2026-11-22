use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::domain::values::roles::NodeRole;
use crate::infra::accounts;

pub struct Organisation {
    pub database: SqliteDatabase,
    pub admin: Account,
    pub member: Account,
    pub rival: Account,
    pub operator: Account,
}

pub async fn organisation() -> Organisation {
    let database = crate::infra::db::open_in_memory().await.expect("open");
    let acme = crate::infra::repositories::tenants::create(&database, "Acme")
        .await
        .expect("tenant");
    let globex = crate::infra::repositories::tenants::create(&database, "Globex")
        .await
        .expect("tenant");

    let admin = person(&database, "admin", NodeRole::Admin, Some(&acme.id)).await;
    let member = person(&database, "member", NodeRole::Member, Some(&acme.id)).await;
    let rival = person(&database, "rival", NodeRole::Admin, Some(&globex.id)).await;
    let operator = person(&database, "operator", NodeRole::Admin, None).await;

    Organisation {
        database,
        admin,
        member,
        rival,
        operator,
    }
}

async fn person(
    database: &SqliteDatabase,
    username: &str,
    role: NodeRole,
    tenant: Option<&str>,
) -> Account {
    accounts::create_in(database, username, None, role, tenant)
        .await
        .expect("account")
}

/// A store of buckets with a disk ceiling, as `CreateStore` would leave it
/// short of its name: what the bucket and key tests start from.
pub async fn store_of_buckets(
    disk: u64,
) -> (SqliteDatabase, crate::domain::entities::service::Service) {
    let database = crate::infra::db::open_in_memory().await.expect("open");
    let service = store_in(&database, disk).await;
    (database, service)
}

/// The same store, `shop/files`, in a database somebody already has.
pub async fn store_in(
    database: &SqliteDatabase,
    disk: u64,
) -> crate::domain::entities::service::Service {
    use crate::domain::entities::service::Kind;
    use crate::infra::repositories::{object_stores, services};

    let project = crate::infra::repositories::projects::create(database, "shop")
        .await
        .expect("project");
    let image = crate::domain::policies::garage::image_for("2").expect("image");
    let service = services::create(database, &project.id, "files", &image, &[])
        .await
        .expect("service");
    services::set_kind(database, &service.id, Kind::Garage)
        .await
        .expect("kind");
    services::set_disk_limit(database, &service.id, Some(disk))
        .await
        .expect("disk");
    object_stores::create(
        database,
        &crate::domain::entities::object_store::ObjectStore {
            service_id: service.id.clone(),
            version: "2".into(),
            rpc_secret: "ab".repeat(32),
            admin_token: "token".into(),
            domain: None,
            restored_from: None,
        },
    )
    .await
    .expect("store");
    services::find(database, &service.id)
        .await
        .expect("read")
        .expect("there")
}

/// Another node on the network, which agreed to keep data for this one and
/// says it runs `version`.
pub async fn far_node(database: &SqliteDatabase, version: Option<&str>) {
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

/// `shop/files`, running on `far.example` rather than here.
pub async fn store_elsewhere(
    disk: u64,
) -> (SqliteDatabase, crate::domain::entities::service::Service) {
    let (database, service) = store_of_buckets(disk).await;
    far_node(&database, Some(crate::infra::network::errand::STORES_SINCE)).await;
    let copy = crate::infra::repositories::replicas::of_service(&database, &service.id)
        .await
        .expect("replicas")[0]
        .clone();
    crate::infra::repositories::replicas::move_to(&database, &copy.id, Some("nd-far"))
        .await
        .expect("moved");
    (database, service)
}

/// What the newest store errand to `far.example` says it keeps.
pub async fn told_far(database: &SqliteDatabase) -> crate::infra::network::errand::Store {
    let record = crate::infra::network::errand::all(database)
        .await
        .expect("errands")
        .into_iter()
        .find(|record| {
            record.node_id == "nd-far" && record.kind == crate::infra::network::errand::Kind::Store
        })
        .expect("a store errand to the far node");
    serde_json::from_value(record.payload).expect("a store errand")
}
