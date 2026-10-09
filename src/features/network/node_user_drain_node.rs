use wabot::sqlite::SqliteDatabase;

use crate::features::network::node_drained_response::NodeDrainedResponse;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::deploy::Deployer;
use crate::network;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    node: &str,
    draining: bool,
) -> Result<NodeDrainedResponse, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    if !draining {
        network::set_draining(database, node, false)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "could not put it back in service");
                "that could not be saved".to_string()
            })?;
        return Ok(NodeDrainedResponse::BackInService);
    }

    let drained = deployer.drain(node).await;
    Ok(NodeDrainedResponse::Moved {
        moved: drained.moved,
        refused: drained.refused,
    })
}
