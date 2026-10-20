use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::capability::Capability;
use crate::features::dns::node_user_add_record::SENT_FROM_ELSEWHERE;
use crate::features::dns::set_zone_servers_request::SetZoneServersRequest;
use crate::infra::repositories::zones;

pub const NOBODY_ANSWERS: &str = "a zone needs at least one node to answer for it";
pub const NOT_AGREED: &str =
    "that node has not agreed to answer DNS for this one — it grants dns from its own console";

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    asked: &SetZoneServersRequest,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let unreadable = |error: String| {
        tracing::warn!(%error, "could not read a zone");
        "the zones could not be read".to_string()
    };
    let zone = zones::find_by_name(database, &asked.zone)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .ok_or_else(|| zones::NO_SUCH_ZONE.to_string())?;
    if !zone.is_ours() {
        return Err(SENT_FROM_ELSEWHERE.into());
    }
    if asked.servers.is_empty() {
        return Err(NOBODY_ANSWERS.into());
    }

    let me = crate::infra::network::all(database)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .into_iter()
        .find(|node| node.is_self)
        .map(|node| node.id);
    let me_first = me.clone();
    let agreed: Vec<String> = crate::infra::network::candidates_for(database, Capability::Dns)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .into_iter()
        .map(|node| node.id)
        .chain(me)
        .collect();
    if asked.servers.iter().any(|server| !agreed.contains(server)) {
        return Err(NOT_AGREED.into());
    }

    let mut servers = asked.servers.clone();
    servers.sort_by_key(|server| Some(server) != me_first.as_ref());

    let before = zones::servers(database, &zone.id)
        .await
        .map_err(|error| unreadable(error.to_string()))?;
    zones::set_servers(database, &zone, &servers)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "could not save who answers for a zone");
            "that could not be saved".to_string()
        })?;
    let removed: Vec<String> = before
        .into_iter()
        .filter(|server| !servers.contains(server))
        .collect();

    crate::infra::dns::reload(container, database).await;
    if let Ok(Some(zone)) = zones::find_by_name(database, &asked.zone).await {
        crate::infra::dns::dispatch::tell_servers(database, &zone, &removed).await;
    }
    Ok(())
}
