use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::features::account::answer_device_request::AnswerDeviceRequest;
use crate::features::account::approve_device_command::ApproveDevice;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &AnswerDeviceRequest,
) -> Result<crate::features::account::device_approved_response::DeviceApprovedResponse, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that account is not this console's".into());
    }
    ApproveDevice { account, asked }.run(database).await
}
