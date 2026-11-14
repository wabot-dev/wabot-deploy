use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::allowlist::Door;
use crate::features::organisations::set_organisation_allowlist_request::SetOrganisationAllowlistRequest;
use crate::infra::deploy::Deployer;
use crate::infra::repositories::{allowlists, tenants};

/// The operator's, like the domain it applies to: who reaches the name the
/// machine serves for an organisation is the machine's side of the
/// arrangement. No guard against an empty list, as the node's own console
/// has — the operator is not behind it, and closing it is a thing an
/// operator may mean.
pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    request: &SetOrganisationAllowlistRequest,
) -> Result<(), String> {
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let Some(tenant) = tenants::find(database, &request.organisation)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such organisation".into());
    };

    allowlists::set(database, &Door::Organisation(tenant.id), &request.allowlist)
        .await
        .map_err(|error| error.to_string())?;
    deployer.sync_routes().await;
    Ok(())
}
