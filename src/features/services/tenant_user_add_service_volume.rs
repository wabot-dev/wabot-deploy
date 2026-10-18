use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::domain::entities::volume::Volume;
use crate::features::services::add_service_volume_request::AddServiceVolumeRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    asked: &AddServiceVolumeRequest,
) -> Result<Volume, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    crate::features::services::add_service_volume_command::AddServiceVolume { service, asked }
        .run(database)
        .await
}
