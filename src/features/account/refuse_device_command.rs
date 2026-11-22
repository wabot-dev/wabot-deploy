use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::domain::values::account_event::AccountEvent;
use crate::features::account::answer_device_request::AnswerDeviceRequest;
use crate::features::account::device_refused_response::DeviceRefusedResponse;
use crate::infra::accounts::{events, requests};

/// "It was not me". The passkey that passed is somebody else's now, so
/// what comes back is which one, for the page to offer removing it.
pub struct RefuseDevice<'a> {
    pub account: &'a Account,
    pub asked: &'a AnswerDeviceRequest,
}

impl RefuseDevice<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<DeviceRefusedResponse, String> {
        let refused = requests::refuse(database, &self.account.id, &self.asked.request_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("that request is closed")?;
        let detail = match &refused.address {
            Some(address) => format!("{} · {address}", refused.name),
            None => refused.name.clone(),
        };
        events::note(
            database,
            &self.account.id,
            AccountEvent::RequestRefused,
            &detail,
            self.asked.address.as_deref(),
            false,
        )
        .await;
        Ok(DeviceRefusedResponse {
            passkey_id: refused.passkey_id,
        })
    }
}
