use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::infra::deploy::Deployer;
use crate::infra::repositories::tenants;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    organisation: &str,
) -> Result<(), String> {
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can do that".into());
    }

    tenants::delete(database, organisation)
        .await
        .map_err(|error| error.to_string())?;

    // Its route goes with it, or the name keeps answering for a frontier
    // that no longer exists.
    deployer.sync_routes().await;
    Ok(())
}
