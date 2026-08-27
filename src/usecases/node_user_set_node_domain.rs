use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::config::Config;
use crate::deploy::Deployer;

pub enum Settled {
    /// The name was checked and saved. The caller has a sentence to show
    /// that the page's own stream cannot: that it resolved *before*
    /// anything was asked of an authority.
    Named(String),
    Cleared,
}

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    deployer: &Deployer,
    account: &Account,
    typed: &str,
) -> Result<Settled, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let previous = crate::node::settings::domain(database, config).await;

    if typed.is_empty() {
        crate::node::settings::set_domain(database, None)
            .await
            .map_err(|error| error.to_string())?;
        refresh(database, config, deployer, previous.as_deref(), None).await;
        return Ok(Settled::Cleared);
    }

    let domain = crate::values::hostname::Hostname::parse(typed)
        .as_str()
        .to_string();
    // Resolved against itself: the node has no other way to know where
    // it is reachable, and "does this name arrive here" is the question.
    let outcome = crate::deploy::dns::resolves_here(&domain, &domain).await;
    if !outcome.ok() {
        return Err(outcome.explain(&domain));
    }

    crate::node::settings::set_domain(database, Some(&domain))
        .await
        .map_err(|error| error.to_string())?;

    // The last failure was about the old name; leaving it would make the
    // page report the new domain as already refused before anything had
    // been asked.
    if let Err(error) = crate::node::settings::set_acme_error(database, None).await {
        tracing::warn!(%error, "could not clear the last certificate failure");
    }
    refresh(
        database,
        config,
        deployer,
        previous.as_deref(),
        Some(&domain),
    )
    .await;

    Ok(Settled::Named(domain))
}

/// The new name has to reach the console and the old one has to stop:
/// the edge answers from the route table, so a rename that only changed
/// the setting would leave the console on a name nobody asked for and
/// off the one they did.
async fn refresh(
    database: &SqliteDatabase,
    config: &Config,
    deployer: &Deployer,
    previous: Option<&str>,
    now: Option<&str>,
) {
    if let Err(error) = crate::network::ensure_self(database, config).await {
        tracing::warn!(%error, "could not refresh this node's own row after a domain change");
    }
    if let Some(previous) = previous.filter(|old| Some(*old) != now) {
        if let Err(error) = crate::edge::routes::forget_control_plane(database, previous).await {
            tracing::warn!(%error, "could not drop the old console route");
        }
    }
    deployer.sync_routes().await;
}
