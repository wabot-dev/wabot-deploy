use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::name_database::{NameDatabase, Named};
use crate::config::Config;
use crate::entities::service::Service;
use crate::platform::projects::Project;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    project: &Project,
    service: &Service,
    typed: &str,
) -> Result<Named, String> {
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
