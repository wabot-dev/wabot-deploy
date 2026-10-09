use wabot::sqlite::SqliteDatabase;

use crate::features::network::node_drained_response::NodeDrainedResponse;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::infra::deploy::Deployer;
use crate::infra::network;

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
