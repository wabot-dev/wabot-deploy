use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::dtos::requests::set_grants::SetGrants;
use crate::network::capability;

/// What **this** node agreed to do, for that one. It lives on the machine
/// the decision binds, and it is what `carry_out` checks before obeying
/// an errand — a grant nothing enforces is decoration.
pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &SetGrants,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    capability::set_granted(database, &asked.node, &asked.agreed)
        .await
        .map_err(|error| {
            let node = &asked.node;
            tracing::warn!(%error, %node, "could not save what this node does for it");
            "that could not be saved".to_string()
        })
}
