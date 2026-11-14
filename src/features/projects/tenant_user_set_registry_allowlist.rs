use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::set_registry_allowlist_command::SetRegistryAllowlist;
use crate::features::projects::set_registry_allowlist_request::SetRegistryAllowlistRequest;
use crate::infra::deploy::Deployer;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    allowed: &Access,
    request: &SetRegistryAllowlistRequest,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    if !allowed.may_administer() {
        return Err("only an owner of this project can choose who reaches its registry".into());
    }

    SetRegistryAllowlist {
        project,
        allowlist: &request.allowlist,
    }
    .run(database, deployer)
    .await
}
