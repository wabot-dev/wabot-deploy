use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::network;
use crate::network::errand::Action;

/// No deduplication and no desired state: asking twice asks twice, and
/// each asking is its own errand with its own id.
pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    node: &str,
    action: Action,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    network::ask_once(database, node, action)
        .await
        .map_err(|error| {
            tracing::warn!(%error, node = %node, "could not ask it");
            error.to_string()
        })
}
