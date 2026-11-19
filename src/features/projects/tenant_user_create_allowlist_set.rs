use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::create_allowlist_set_command::CreateAllowlistSet;
use crate::features::projects::create_allowlist_set_request::CreateAllowlistSetRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
    allowed: &Access,
    request: &CreateAllowlistSetRequest,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }
    if !allowed.may_deploy() {
        return Err(
            "only somebody who may deploy in this project can change what its services share"
                .into(),
        );
    }

    CreateAllowlistSet {
        project,
        name: &request.name,
        list: &request.list,
    }
    .run(database)
    .await
}
