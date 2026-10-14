use wabot::sqlite::SqliteDatabase;

use crate::features::node::domain_settled_response::DomainSettledResponse;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::infra::config::Config;
use crate::infra::deploy::dns::Resolution;
use crate::infra::deploy::Deployer;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    deployer: &Deployer,
    account: &Account,
    typed: &str,
) -> Result<DomainSettledResponse, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let previous = crate::infra::node::settings::domain(database).await;

    if typed.is_empty() {
        crate::infra::node::settings::set_domain(database, None)
            .await
            .map_err(|error| error.to_string())?;
        refresh(database, config, deployer, previous.as_deref(), None).await;
        return Ok(DomainSettledResponse::Cleared);
    }

    let parsed = crate::domain::values::hostname::Hostname::parse(typed);
    if parsed.is_wildcard() {
        return Err(crate::domain::values::hostname::WILDCARD_REFUSED.into());
    }
    let domain = parsed.as_str().to_string();
    // Resolved against itself: the node has no other way to know where
    // it is reachable, and "does this name arrive here" is the question.
    // So a name that resolves nowhere is also the reference that resolves
    // nowhere, and the sentence for that one told somebody typing a
    // domain to go and give the node a domain.
    let outcome = match crate::infra::deploy::dns::resolves_here(&domain, &domain).await {
        Resolution::NoReference => Resolution::Missing,
        outcome => outcome,
    };
    if !outcome.ok() {
        return Err(outcome.explain(&domain));
    }

    crate::infra::node::settings::set_domain(database, Some(&domain))
        .await
        .map_err(|error| error.to_string())?;

    // The last failure was about the old name; leaving it would make the
    // page report the new domain as already refused before anything had
    // been asked.
    if let Err(error) = crate::infra::node::settings::set_acme_error(database, None).await {
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

    let addresses = crate::infra::deploy::dns::lookup(&domain)
        .await
        .iter()
        .map(ToString::to_string)
        .collect();
    Ok(DomainSettledResponse::Named { domain, addresses })
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
    if let Err(error) = crate::infra::network::ensure_self(database, config).await {
        tracing::warn!(%error, "could not refresh this node's own row after a domain change");
    }
    if let Some(previous) = previous.filter(|old| Some(*old) != now) {
        if let Err(error) =
            crate::infra::edge::routes::forget_control_plane(database, previous).await
        {
            tracing::warn!(%error, "could not drop the old console route");
        }
    }
    deployer.sync_routes().await;
}
