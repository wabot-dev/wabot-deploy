use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::domain::values::account_event::AccountEvent;
use crate::features::account::remove_passkey_request::RemovePasskeyRequest;
use crate::infra::accounts::{events, passkeys};

/// The last one may go too: "It was not me" offers exactly that, and a
/// link is the way back in.
pub struct RemovePasskey<'a> {
    pub account: &'a Account,
    pub asked: &'a RemovePasskeyRequest,
}

impl RemovePasskey<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        let found = passkeys::find(database, &self.asked.passkey_id)
            .await
            .map_err(|error| error.to_string())?
            .filter(|passkey| passkey.account_id == self.account.id)
            .ok_or("that passkey is not one of yours")?;
        passkeys::delete(database, &self.account.id, &found.id)
            .await
            .map_err(|error| error.to_string())?;
        events::note(
            database,
            &self.account.id,
            AccountEvent::PasskeyRemoved,
            found.title(),
            self.asked.address.as_deref(),
            false,
        )
        .await;
        Ok(())
    }
}
