use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::features::account::answer_device_request::AnswerDeviceRequest;
use crate::features::account::refuse_device_command::RefuseDevice;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &AnswerDeviceRequest,
) -> Result<crate::features::account::device_refused_response::DeviceRefusedResponse, String> {
    if !node_user::belongs_here(account) {
        return Err("that account is not this console's".into());
    }
    RefuseDevice { account, asked }.run(database).await
}
