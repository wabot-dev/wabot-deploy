use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::set_service_image::SetServiceImage;
use crate::platform::services::Service;

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
