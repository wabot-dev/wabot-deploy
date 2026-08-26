use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::choose_env_revision::ChooseEnvRevision;
use crate::platform::services::Service;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    revision_id: &str,
) -> Result<Vec<(String, String)>, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    ChooseEnvRevision {
        service,
        revision_id,
    }
    .run(database)
    .await
}
