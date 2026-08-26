use std::collections::HashMap;

use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::config::Config;
use crate::network;
use crate::network::capability::Capability;

pub enum Spent {
    /// The terms were read but nothing agreed to yet. The token comes
    /// back so the console can show what it asks for **before** it is
    /// spent, which is the whole point of the order.
    Review(String),
    /// Named, because the page says who this node now answers to.
    Joined(String),
}

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    token: &str,
    ticked: &HashMap<String, String>,
) -> Result<Spent, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if token.is_empty() {
        return Err("paste the join token".into());
    }

    if !ticked.contains_key("agreed") {
        network::token::JoinToken::decode(token).map_err(|error| error.to_string())?;
        return Ok(Spent::Review(token.to_string()));
    }

    let accepted: Vec<Capability> = Capability::ALL
        .into_iter()
        .filter(|capability| ticked.contains_key(&format!("accept-{}", capability.name())))
        .collect();

    let joined = network::join::join(database, config, token, Some(&accepted))
        .await
        .map_err(|error| error.to_string())?;
    Ok(Spent::Joined(joined.authority.name))
}
