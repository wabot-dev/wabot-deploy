use wabot::sqlite::SqliteDatabase;

use crate::authz::node_user;
use crate::config::Config;
use crate::entities::account::Account;
use crate::features::network::mint_join_token_request::MintJoinTokenRequest;
use crate::network;
use crate::values::capability::Capability;

/// **A join states its terms, and the joiner reads them first.** The
/// token carries what this node *requires* and what it *offers*, per
/// capability, so the joining console can show both lists before the
/// token is spent. Terms shown after committing are a consent screen for
/// a decision already made.
pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    asked: &MintJoinTokenRequest,
) -> Result<String, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if asked.name.chars().count() > 60 {
        return Err("that note is too long for the column it goes in".into());
    }

    let me = network::ensure_hub(database, config)
        .await
        .map_err(|error| error.to_string())?;
    let Some(endpoint) = me.endpoint.clone().filter(|_| me.may_be_edge()) else {
        return Err(
            "this node has no address another one could call back on — set a domain first".into(),
        );
    };
    let Some(public_key) = network::keys::public_key(database).await else {
        return Err("this node has no overlay key yet".into());
    };
    let Some(overlay_ip) = me.overlay_ip.clone() else {
        return Err("this node has no address on its own overlay yet".into());
    };

    let assigned_ip = network::overlay::allocate(database)
        .await
        .map_err(|error| error.to_string())?;
    let (_, secret) = network::enrolment::create(
        database,
        &asked.name,
        &assigned_ip,
        &account.id,
        &asked.requires,
        &asked.offers,
    )
    .await
    .map_err(|error| error.to_string())?;

    Ok(network::token::JoinToken {
        authority: me.id,
        name: me.name,
        endpoint,
        public_key,
        overlay_ip,
        assigned_ip,
        secret,
        requires: Some(names(&asked.requires)),
        offers: Some(names(&asked.offers)),
    }
    .encode())
}

fn names(capabilities: &[Capability]) -> Vec<String> {
    capabilities
        .iter()
        .map(|capability| capability.name().to_string())
        .collect()
}
