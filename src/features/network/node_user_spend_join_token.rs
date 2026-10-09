use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::config::Config;
use crate::features::network::join_token_spent_response::JoinTokenSpentResponse;
use crate::features::network::spend_join_token_request::SpendJoinTokenRequest;
use crate::network;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    asked: &SpendJoinTokenRequest,
) -> Result<JoinTokenSpentResponse, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if asked.token.is_empty() {
        return Err("paste the join token".into());
    }

    if !asked.agreed {
        network::token::JoinToken::decode(&asked.token).map_err(|error| error.to_string())?;
        return Ok(JoinTokenSpentResponse::Review(asked.token.clone()));
    }

    let joined = network::join::join(database, config, &asked.token, Some(&asked.accepted))
        .await
        .map_err(|error| error.to_string())?;
    Ok(JoinTokenSpentResponse::Joined(joined.authority.name))
}
