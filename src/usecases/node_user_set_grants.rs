use std::collections::HashMap;

use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::network::capability::{self, Capability};

/// What **this** node agreed to do, for that one. It lives on the machine
/// the decision binds, and it is what `carry_out` checks before obeying
/// an errand — a grant nothing enforces is decoration.
pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    node: &str,
    ticked: &HashMap<String, String>,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let agreed: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| ticked.contains_key(capability.name()))
        .collect();

    capability::set_granted(database, node, &agreed)
        .await
        .map_err(|error| {
            tracing::warn!(%error, node = %node, "could not save what this node does for it");
            "that could not be saved".to_string()
        })
}
