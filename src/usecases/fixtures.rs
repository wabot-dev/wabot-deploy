use wabot::sqlite::SqliteDatabase;

use crate::accounts::roles::NodeRole;
use crate::accounts::{self, Account};

pub struct Organisation {
    pub database: SqliteDatabase,
    pub admin: Account,
    pub member: Account,
    pub rival: Account,
    pub operator: Account,
}

pub async fn organisation() -> Organisation {
    let database = crate::db::open_in_memory().await.expect("open");
    let acme = crate::platform::tenants::create(&database, "Acme")
        .await
        .expect("tenant");
    let globex = crate::platform::tenants::create(&database, "Globex")
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
