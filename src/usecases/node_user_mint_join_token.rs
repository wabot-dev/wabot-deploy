use std::collections::HashMap;

use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::config::Config;
use crate::network;
use crate::network::capability::Capability;

/// **A join states its terms, and the joiner reads them first.** The
/// token carries what this node *requires* and what it *offers*, per
/// capability, so the joining console can show both lists before the
/// token is spent. Terms shown after committing are a consent screen for
/// a decision already made.
pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    name: &str,
    ticked: &HashMap<String, String>,
) -> Result<String, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if name.chars().count() > 60 {
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

    let requires: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| ticked.contains_key(&format!("require-{}", capability.name())))
        .collect();
    let offers: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| ticked.contains_key(&format!("offer-{}", capability.name())))
        .collect();

    let assigned_ip = network::overlay::allocate(database)
        .await
        .map_err(|error| error.to_string())?;
    let (_, secret) = network::enrolment::create(
        database,
        name,
        &assigned_ip,
        &account.id,
        &requires,
        &offers,
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
        requires: Some(names(&requires)),
        offers: Some(names(&offers)),
    }
    .encode())
}

fn names(capabilities: &[Capability]) -> Vec<String> {
    capabilities
        .iter()
        .map(|capability| capability.name().to_string())
        .collect()
}
