use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::account::add_device_request::AddDeviceRequest;
use crate::features::account::approve_device_command::NOT_FROM_HERE;
use crate::features::account::device_link_response::DeviceLinkResponse;
use crate::infra::accounts::links::{self, Purpose};

/// A link the phone opens to make its own passkey and be trusted: what a
/// device with no passkey yet needs.
pub struct AddDevice<'a> {
    pub account: &'a Account,
    pub asked: &'a AddDeviceRequest,
}

impl AddDevice<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<DeviceLinkResponse, String> {
        if !self.asked.trusted {
            return Err(NOT_FROM_HERE.into());
        }
        let token = links::issue(
            database,
            &self.account.id,
            Purpose::Device,
            Some((&self.account.id, &self.account.username)),
        )
        .await
        .map_err(|error| error.to_string())?;
        Ok(DeviceLinkResponse {
            token,
            expires_at: crate::infra::clock::now_ms() + Purpose::Device.minutes() * 60_000,
        })
    }
}
