use wabot::sqlite::SqliteDatabase;

use crate::authz::node_user;
use crate::config::Config;
use crate::entities::account::Account;
use crate::entities::project::Project;
use crate::entities::service::Service;
use crate::features::databases::database_named_response::DatabaseNamedResponse;
use crate::features::databases::name_database_command::NameDatabase;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    project: &Project,
    service: &Service,
    typed: &str,
) -> Result<DatabaseNamedResponse, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to name".into());
    }

    NameDatabase {
        project,
        service,
        typed,
    }
    .run(database, config)
    .await
}
