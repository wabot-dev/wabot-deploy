use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::features::account::add_device_command::AddDevice;
use crate::features::account::add_device_request::AddDeviceRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &AddDeviceRequest,
) -> Result<crate::features::account::device_link_response::DeviceLinkResponse, String> {
    if !node_user::belongs_here(account) {
        return Err("that account is not this console's".into());
    }
    AddDevice { account, asked }.run(database).await
}
