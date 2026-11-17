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
    accounts::create_in(database, username, "a long passphrase here", role, tenant)
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
    let service = services::create_on(database, &project.id, "files", &image, &[], None)
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
