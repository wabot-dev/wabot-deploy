use wabot::sqlite::SqliteDatabase;
use wabot_addon_auth_passkey::RelyingParty;

use crate::domain::entities::account::Account;
use crate::domain::values::account_event::AccountEvent;
use crate::domain::values::browser::Browser;
use crate::features::access::signed_in_response::SignedInResponse;
use crate::infra::accounts::ceremonies::Made;
use crate::infra::accounts::{self, devices, events, passkeys, sessions};

/// Somebody proved who they are: keep the passkey they made, forget the
/// password it replaces, trust the browser if they said to, and sign them
/// in on it.
pub struct Welcome<'a> {
    pub account: &'a Account,
    pub passkey: Option<(&'a RelyingParty, &'a Made)>,
    pub browser: &'a Browser,
    pub trust: bool,
    /// Whether this happened somewhere the owner was not looking.
    pub news: bool,
}

impl Welcome<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<SignedInResponse, String> {
        let (account, browser) = (self.account, self.browser);
        let address = browser.address.as_deref();
        let mut passkey_id = None;
        if let Some((rp, made)) = self.passkey {
            let stored = passkeys::insert(
                database,
                &account.id,
                &rp.id,
                &made.registered,
                &browser.name,
            )
            .await
            .map_err(|error| error.to_string())?;
            events::note(
                database,
                &account.id,
                AccountEvent::PasskeyAdded,
                stored.title(),
                address,
                self.news,
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
            passkey_id = Some(stored.id);
        }

        let trusted = match self.trust {
            true => {
                let (id, token) = devices::trust(
                    database,
                    browser.device.as_deref(),
                    &account.id,
                    &browser.name,
                    passkey_id.as_deref(),
                    address,
                )
                .await
                .map_err(|error| error.to_string())?;
                events::note(
                    database,
                    &account.id,
                    AccountEvent::DeviceTrusted,
                    &browser.name,
                    address,
                    self.news,
                )
                .await;
                Some((id, token))
            }
            false => devices::recognise(database, browser.device.as_deref(), &account.id, address)
                .await
                .map_err(|error| error.to_string())?
                .map(|id| (id, String::new())),
        };

        let session = sessions::start(
            database,
            account,
            trusted.as_ref().map(|(id, _)| id.as_str()),
        )
        .await
        .map_err(|error| error.to_string())?;
        Ok(SignedInResponse {
            session,
            lasting: trusted.is_some(),
            device: trusted
                .map(|(_, token)| token)
                .filter(|token| !token.is_empty()),
        })
    }
}
