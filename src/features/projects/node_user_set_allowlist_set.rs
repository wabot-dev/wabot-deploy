use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::set_allowlist_set_command::SetAllowlistSet;
use crate::features::projects::set_allowlist_set_request::SetAllowlistSetRequest;
use crate::infra::deploy::Deployer;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    allowed: &Access,
    name: &str,
    request: &SetAllowlistSetRequest,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }
    if !allowed.may_deploy() {
        return Err(
            "only somebody who may deploy in this project can change what its services share"
                .into(),
        );
    }

    SetAllowlistSet {
        project,
        name,
        list: &request.list,
    }
    .run(database, deployer)
    .await
}
