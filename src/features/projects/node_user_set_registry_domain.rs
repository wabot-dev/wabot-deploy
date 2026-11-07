use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::set_registry_domain_command::SetRegistryDomain;
use crate::features::projects::set_registry_domain_request::SetRegistryDomainRequest;
use crate::infra::deploy::Deployer;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    allowed: &Access,
    request: &SetRegistryDomainRequest,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    if !allowed.may_administer() {
        return Err("only an owner of this project can choose where it is pushed".into());
    }

    SetRegistryDomain {
        project,
        domain: request.domain.as_deref(),
    }
    .run(database, deployer)
    .await
}
