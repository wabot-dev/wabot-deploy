use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::deploy::Deployer;
use crate::network;

pub enum Drained {
    BackInService,
    /// Draining is **convergent, not atomic**: a node that is not
    /// answering keeps its containers until it collects the errand with
    /// the slot gone, so the count and the refusals are both worth
    /// showing.
    Moved {
        moved: usize,
        refused: Vec<String>,
    },
}

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    node: &str,
    draining: bool,
) -> Result<Drained, String> {
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
        return Ok(Drained::BackInService);
    }

    let drained = deployer.drain(node).await;
    Ok(Drained::Moved {
        moved: drained.moved,
        refused: drained.refused,
    })
}
