//! Where one name's certificate is to come from.
//!
//! **A command, because three controllers reuse it**: the node's own
//! domain, a managed database's name and a service's hostname all offer
//! the same choice and all write it the same way. It lived beside one of
//! them and the other two reached across for it, which is what splitting
//! that file into its two controllers made impossible to leave alone.

/// Store it — **forgetting** rather than writing the default.
///
/// A stored default would go stale the day `acme.disabled` changed, and
/// then say the opposite of what the node does.
pub async fn run(
    database: &wabot::sqlite::SqliteDatabase,
    config: &crate::config::Config,
    name: &str,
    renew_with: &crate::edge::policy::RenewWith,
) -> Result<(), String> {
    let stored = match renew_with == &crate::edge::policy::RenewWith::default_for(config) {
        true => crate::edge::policy::clear(database, name).await,
        false => crate::edge::policy::set(database, name, renew_with).await,
    };
    stored.map_err(|error| error.to_string())
}
