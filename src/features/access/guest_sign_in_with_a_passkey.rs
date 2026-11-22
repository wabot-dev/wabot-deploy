use wabot::sqlite::SqliteDatabase;

use crate::features::access::passkey_sign_in_request::PasskeySignInRequest;
use crate::features::access::passkey_sign_in_response::PasskeySignInResponse;
use crate::features::access::welcome_command::Welcome;
use crate::infra::accounts::challenges::Purpose;
use crate::infra::accounts::{self, ceremonies, devices, requests};

pub const NOBODY_TO_ASK: &str = "no device of yours is trusted to let this one in — ask whoever \
                                 runs this console for a sign-in link";

/// A passkey is enough on a device the account trusts. On any other it
/// opens a request a trusted device has to approve, because a passkey
/// syncs to every device of whoever controls the cloud account behind it.
pub async fn run(
    database: &SqliteDatabase,
    asked: &PasskeySignInRequest,
) -> Result<PasskeySignInResponse, String> {
    let rp = ceremonies::relying_party(&asked.browser.host).ok_or(ceremonies::NO_PASSKEYS_HERE)?;
    let passkey =
        ceremonies::finish_assertion(database, &rp, &asked.passkey, Purpose::SignIn, None).await?;
    let account = accounts::find(database, &passkey.account_id)
        .await
        .map_err(|error| error.to_string())?
        .filter(|account| account.belongs_to(asked.scope.as_deref()))
        .ok_or(ceremonies::UNKNOWN)?;
    let address = asked.browser.address.as_deref();

    let trusted = devices::recognise(
        database,
        asked.browser.device.as_deref(),
        &account.id,
        address,
    )
    .await
    .map_err(|error| error.to_string())?;
    if trusted.is_some() {
        return Welcome {
            account: &account,
            passkey: None,
            browser: &asked.browser,
            trust: false,
            news: false,
        }
        .run(database)
        .await
        .map(PasskeySignInResponse::SignedIn);
    }

    if devices::of_account(database, &account.id)
        .await
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Err(NOBODY_TO_ASK.into());
    }
    requests::open(
        database,
        &account.id,
        &passkey.id,
        &asked.browser.name,
        address,
    )
    .await
    .map(PasskeySignInResponse::Awaiting)
    .map_err(|error| error.to_string())
}
