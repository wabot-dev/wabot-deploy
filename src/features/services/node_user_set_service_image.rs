use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::features::services::set_service_image_command::SetServiceImage;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    image: &str,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }

    SetServiceImage { service, image }.run(database).await
}
