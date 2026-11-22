use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::features::account::withdraw_device_command::WithdrawDevice;
use crate::features::account::withdraw_device_request::WithdrawDeviceRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &WithdrawDeviceRequest,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that account is not this console's".into());
    }
    WithdrawDevice { account, asked }.run(database).await
}
