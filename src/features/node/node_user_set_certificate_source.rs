use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::infra::config::Config;

/// Where this node's own name gets its certificate from.
///
/// The name comes from the setting rather than the form: a source stored
/// against a name nobody serves is an answer nothing reads.
pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
) -> Result<String, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    Ok(crate::infra::node::settings::domain(database, config)
        .await
        .unwrap_or_else(|| crate::infra::edge::certs::FALLBACK_NAME.to_string()))
}
