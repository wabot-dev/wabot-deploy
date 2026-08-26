use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::commands::check_database_certificate::CheckDatabaseCertificate;
use crate::config::Config;
use crate::edge::policy::RenewWith;
use crate::platform::projects::Project;
use crate::platform::services::Service;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    project: &Project,
    service: &Service,
    renew_with: &RenewWith,
) -> Result<String, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to set".into());
    }

    CheckDatabaseCertificate {
        project,
        service,
        renew_with,
    }
    .run(database, config)
    .await
}
