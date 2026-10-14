use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;

pub async fn run(database: &SqliteDatabase, account: &Account, on: bool) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    crate::infra::node::settings::set_acme_enabled(database, on)
        .await
        .map_err(|error| error.to_string())
}
