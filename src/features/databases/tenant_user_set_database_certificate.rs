use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::features::databases::check_database_certificate_command::CheckDatabaseCertificate;
use crate::infra::edge::policy::RenewWith;

pub async fn run(
    database: &SqliteDatabase,
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
    .run(database)
    .await
}
