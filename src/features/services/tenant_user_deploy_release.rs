use wabot::sqlite::SqliteDatabase;

use crate::authz::tenant_user;
use crate::entities::account::Account;
use crate::entities::release::Release;
use crate::entities::service::Service;
use crate::features::services::choose_release_command::ChooseRelease;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    release_id: &str,
) -> Result<Release, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to deploy".into());
    }

    ChooseRelease {
        service,
        release_id,
    }
    .run(database)
    .await
}
