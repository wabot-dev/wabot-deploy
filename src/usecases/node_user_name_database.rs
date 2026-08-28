use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::name_database::NameDatabase;
use crate::config::Config;
use crate::dtos::responses::database_named::DatabaseNamed;
use crate::entities::project::Project;
use crate::entities::service::Service;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    project: &Project,
    service: &Service,
    typed: &str,
) -> Result<DatabaseNamed, String> {
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
