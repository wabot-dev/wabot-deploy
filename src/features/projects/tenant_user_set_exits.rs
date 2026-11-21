use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::set_exits_command::SetExits;
use crate::features::projects::set_exits_request::SetExitsRequest;
use crate::infra::deploy::Deployer;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    project: &Project,
    allowed: &Access,
    request: &SetExitsRequest,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    if !allowed.may_administer() {
        return Err("only an owner of this project can choose where it leaves from".into());
    }

    SetExits {
        project,
        exits: &request.exits,
    }
    .run(database, deployer)
    .await
}
