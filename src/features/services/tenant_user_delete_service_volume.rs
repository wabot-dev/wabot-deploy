use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::features::services::delete_service_volume_command::DeleteServiceVolume;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    volume_id: &str,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    DeleteServiceVolume { service, volume_id }
        .run(database)
        .await
}
