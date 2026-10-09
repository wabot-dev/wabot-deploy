use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::capability::Capability;
use crate::features::node::set_capabilities_request::SetCapabilitiesRequest;
use crate::infra::config::Config;
use crate::infra::deploy::Deployer;
use crate::infra::network::capability;

/// **The one POST under `/node` that had no admin check**, and the
/// omission was not small: `evict_ungranted` and `release_ungranted` run
/// at boot and act on exactly these values, so turning `host` off throws
/// off every replica this node holds for anybody, and turning `edge` off
/// releases every name it claims. The page drawing the form was already
/// behind the check, which made it hidden and not enforced.
pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    deployer: &Deployer,
    account: &Account,
    asked: &SetCapabilitiesRequest,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    for capability in Capability::ALL {
        let wanted = asked.provides.contains(&capability);
        capability::set_provides(database, capability, wanted)
            .await
            .map_err(|error| {
                tracing::warn!(%error, capability = capability.name(), "could not save it");
                "that could not be saved".to_string()
            })?;
    }

    // Whether this node originates anything is not one of them — see
    // `capability::owns`.
    //
    // **Refused while it still owns something.** A node that stopped
    // being an owner with projects still in it would leave every one of
    // them in a state nothing can act on: not editable, because this node
    // no longer originates; not evictable, because they are its own. The
    // message names the count so it is a thing somebody can go and do.
    let owns = asked.owns;
    let mine = crate::infra::repositories::projects::all(database)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "could not count this node's own projects");
            "that could not be saved".to_string()
        })?
        .into_iter()
        .filter(|project| project.is_ours())
        .count();
    if !owns && mine > 0 {
        return Err(crate::shell::texts::sentences::still_owns_projects(mine));
    }
    capability::set_owns(database, owns)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "could not save whether this node owns anything");
            "that could not be saved".to_string()
        })?;

    // The self row carries the answer: `kind` is derived from the edge
    // capability, and every other node learns this one's kind from the
    // endpoint it reports. Rewriting it now is what makes the change
    // travel rather than wait for a restart.
    if let Err(error) = crate::infra::network::ensure_self(database, config).await {
        tracing::warn!(%error, "could not bring this node's own row up to date");
    }
    // And the routes: a node that has just stopped being an edge is still
    // answering for every name it held a moment ago.
    deployer.sync_routes().await;

    Ok(())
}
