use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::infra::repositories::zones;

/// A zone sent by another node is deleted here too, and that is eviction:
/// the operator of a machine may always put down what somebody else asked
/// it to carry.
pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    zone_name: &str,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let zone = zones::find_by_name(database, zone_name)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "could not read a zone");
            "the zones could not be read".to_string()
        })?
        .ok_or_else(|| zones::NO_SUCH_ZONE.to_string())?;

    let servers = zones::servers(database, &zone.id).await.unwrap_or_default();
    zones::delete(database, &zone).await.map_err(|error| {
        tracing::warn!(%error, "could not delete a zone");
        "that could not be saved".to_string()
    })?;

    crate::infra::dns::reload(container, database).await;
    crate::infra::dns::dispatch::tell_servers(database, &zone, &servers).await;
    Ok(())
}
