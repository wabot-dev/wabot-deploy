use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::domain::values::account_event::AccountEvent;
use crate::features::account::withdraw_device_request::WithdrawDeviceRequest;
use crate::infra::accounts::{devices, events};

/// Stop trusting a device, which ends every session made on it.
pub struct WithdrawDevice<'a> {
    pub account: &'a Account,
    pub asked: &'a WithdrawDeviceRequest,
}

impl WithdrawDevice<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        let found = devices::of_account(database, &self.account.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|device| device.id == self.asked.device_id)
            .ok_or("that device is not one of yours")?;
        devices::withdraw(database, &self.account.id, &found.id)
            .await
            .map_err(|error| error.to_string())?;
        events::note(
            database,
            &self.account.id,
            AccountEvent::DeviceWithdrawn,
            &found.name,
            self.asked.address.as_deref(),
            false,
        )
        .await;
        Ok(())
    }
}
