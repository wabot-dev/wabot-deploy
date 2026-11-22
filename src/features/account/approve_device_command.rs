use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::domain::values::account_event::AccountEvent;
use crate::features::account::answer_device_request::AnswerDeviceRequest;
use crate::features::account::device_approved_response::DeviceApprovedResponse;
use crate::infra::accounts::{events, requests};

pub const NOT_FROM_HERE: &str =
    "only a device you trust can let another in, and this one is not trusted";

/// "It was me": the code the new device types. Shown here and typed
/// there, so nobody approves by accident and the new device needs nothing
/// but a form.
pub struct ApproveDevice<'a> {
    pub account: &'a Account,
    pub asked: &'a AnswerDeviceRequest,
}

impl ApproveDevice<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<DeviceApprovedResponse, String> {
        if !self.asked.trusted {
            return Err(NOT_FROM_HERE.into());
        }
        let pending = requests::pending_for(database, &self.account.id)
            .await
            .map_err(|error| error.to_string())?
            .filter(|pending| pending.id == self.asked.request_id)
            .ok_or("that request is closed")?;
        let code = requests::approve(database, &self.account.id, &pending.id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("that request is closed")?;
        events::note(
            database,
            &self.account.id,
            AccountEvent::RequestApproved,
            &pending.name,
            pending.address.as_deref(),
            false,
        )
        .await;
        Ok(DeviceApprovedResponse {
            code,
            name: pending.name,
        })
    }
}
