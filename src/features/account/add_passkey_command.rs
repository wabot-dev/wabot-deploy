use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::domain::values::account_event::AccountEvent;
use crate::features::account::add_passkey_request::AddPasskeyRequest;
use crate::features::account::passkey_added_response::PasskeyAddedResponse;
use crate::infra::accounts::{self, ceremonies, devices, events, passkeys, sessions};

/// Keep a passkey the account made. The first one also deletes the
/// password it replaces and, when no device is trusted yet, trusts this
/// one: that is how an account from before passkeys arrives.
pub struct AddPasskey<'a> {
    pub account: &'a Account,
    pub asked: &'a AddPasskeyRequest,
}

impl AddPasskey<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<PasskeyAddedResponse, String> {
        let (account, asked) = (self.account, self.asked);
        let address = asked.browser.address.as_deref();
        let rp =
            ceremonies::relying_party(&asked.browser.host).ok_or(ceremonies::NO_PASSKEYS_HERE)?;
        let made =
            ceremonies::finish_registration(database, &rp, &asked.passkey, Some(&account.id))
                .await?;
        let stored = passkeys::insert(
            database,
            &account.id,
            &rp.id,
            &made.registered,
            &asked.browser.name,
        )
        .await
        .map_err(|error| error.to_string())?;
        events::note(
            database,
            &account.id,
            AccountEvent::PasskeyAdded,
            stored.title(),
            address,
            false,
        )
        .await;
        if accounts::clear_password(database, &account.id)
            .await
            .map_err(|error| error.to_string())?
        {
            events::note(
                database,
                &account.id,
                AccountEvent::PasswordDeleted,
                "",
                address,
                false,
            )
            .await;
        }

        let trusted = devices::of_account(database, &account.id)
            .await
            .map_err(|error| error.to_string())?;
        if !trusted.is_empty() {
            return Ok(PasskeyAddedResponse { device: None });
        }
        let (id, token) = devices::trust(
            database,
            asked.browser.device.as_deref(),
            &account.id,
            &asked.browser.name,
            Some(&stored.id),
            address,
        )
        .await
        .map_err(|error| error.to_string())?;
        sessions::bind(database, &asked.session, &id)
            .await
            .map_err(|error| error.to_string())?;
        events::note(
            database,
            &account.id,
            AccountEvent::DeviceTrusted,
            &asked.browser.name,
            address,
            false,
        )
        .await;
        Ok(PasskeyAddedResponse {
            device: Some(token),
        })
    }
}
