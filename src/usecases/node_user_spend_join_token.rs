use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::config::Config;
use crate::dtos::requests::spend_join_token::SpendJoinToken;
use crate::dtos::responses::join_token_spent::Spent;
use crate::network;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    asked: &SpendJoinToken,
) -> Result<Spent, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if asked.token.is_empty() {
        return Err("paste the join token".into());
    }

    if !asked.agreed {
        network::token::JoinToken::decode(&asked.token).map_err(|error| error.to_string())?;
        return Ok(Spent::Review(asked.token.clone()));
    }

    let joined = network::join::join(database, config, &asked.token, Some(&asked.accepted))
        .await
        .map_err(|error| error.to_string())?;
    Ok(Spent::Joined(joined.authority.name))
}
