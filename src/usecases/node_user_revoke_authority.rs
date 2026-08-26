use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::network;

pub async fn run(database: &SqliteDatabase, account: &Account, node: &str) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    network::revoke(database, node)
        .await
        .map_err(|error| error.to_string())
}
