use wabot::sqlite::SqliteDatabase;

use crate::authz::tenant_user;
use crate::config::Config;
use crate::edge::policy::RenewWith;
use crate::entities::account::Account;
use crate::entities::project::Project;
use crate::entities::service::Service;
use crate::features::databases::check_database_certificate_command::CheckDatabaseCertificate;

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
