use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::name_server::NameServerName;
use crate::features::dns::set_name_servers_request::SetNameServersRequest;
use crate::infra::dns::names;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    asked: &SetNameServersRequest,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let named = NameServerName::all(&asked.names).map_err(str::to_string)?;
    names::declare(database, &named).await.map_err(|error| {
        tracing::warn!(%error, "could not save the names this node answers DNS as");
        "that could not be saved".to_string()
    })?;
    names::changed(container, database).await;
    Ok(())
}
