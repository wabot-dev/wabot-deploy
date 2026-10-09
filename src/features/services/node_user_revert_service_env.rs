use wabot::sqlite::SqliteDatabase;

use crate::authz::node_user;
use crate::entities::account::Account;
use crate::entities::service::Service;
use crate::features::services::choose_env_revision_command::ChooseEnvRevision;

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
