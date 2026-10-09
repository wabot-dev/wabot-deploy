use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::infra::network;

/// "Be on this release" is desired state, so it is an ordinary errand on
/// the pull path. A node already there settles it as satisfied; one that
/// was off converges when it comes back.
pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    node: &str,
    wanted: &str,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let wanted = wanted.trim();
    let wanted = (!wanted.is_empty()).then_some(wanted);
    network::want_version(database, node, wanted)
        .await
        .map_err(|error| {
            tracing::warn!(%error, node = %node, "could not record the release to ask for");
            error.to_string()
        })?;

    // Now rather than at the next boot, so the button does something
    // somebody can see. The errand is the same one either way.
    network::dispatch_versions(database)
        .await
        .map_err(|error| {
            tracing::warn!(%error, node = %node, "could not queue the instruction");
            error.to_string()
        })?;
    Ok(())
}
