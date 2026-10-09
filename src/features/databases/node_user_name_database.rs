use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::features::databases::database_named_response::DatabaseNamedResponse;
use crate::features::databases::name_database_command::NameDatabase;
use crate::infra::config::Config;

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
