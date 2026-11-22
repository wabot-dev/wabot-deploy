use wabot::sqlite::SqliteDatabase;

use crate::domain::values::account_event::AccountEvent;
use crate::features::access::signed_in_response::SignedInResponse;
use crate::features::access::use_link_request::UseLinkRequest;
use crate::features::access::welcome_command::Welcome;
use crate::infra::accounts::links::{self, Purpose};
use crate::infra::accounts::{self, ceremonies, events};

pub const NOT_VALID: &str = "that link is not valid — it may have been used already, or expired";

/// A link signs its holder in and counts as the second factor. A device
/// link, from a trusted device's "Add a device", always trusts the
/// browser it is opened in; a sign-in link does when they say so.
pub async fn run(
    database: &SqliteDatabase,
    asked: &UseLinkRequest,
) -> Result<SignedInResponse, String> {
    let link = links::look_up(database, &asked.token)
        .await
        .map_err(|error| error.to_string())?
        .ok_or(NOT_VALID)?;
    let account = accounts::find(database, &link.account_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or(NOT_VALID)?;
    if !account.belongs_to(asked.scope.as_deref()) {
        return Err("that link is for another console — open it on the address it names".into());
    }

    let made = match asked.passkey.given() {
        true => {
            let rp = ceremonies::relying_party(&asked.browser.host)
                .ok_or(ceremonies::NO_PASSKEYS_HERE)?;
            let made =
                ceremonies::finish_registration(database, &rp, &asked.passkey, Some(&account.id))
                    .await?;
            Some((rp, made))
        }
        false => None,
    };

    let link = links::spend(database, &asked.token)
        .await
        .map_err(|error| error.to_string())?
        .ok_or(NOT_VALID)?;
    let issuer = link.issued_by_name.as_deref().unwrap_or("root");
    events::note(
        database,
        &account.id,
        AccountEvent::LinkUsed,
        issuer,
        asked.browser.address.as_deref(),
        true,
    )
    .await;

    Welcome {
        account: &account,
        passkey: made.as_ref().map(|(rp, made)| (rp, made)),
        browser: &asked.browser,
        trust: asked.trust || link.purpose == Purpose::Device,
        news: true,
    }
    .run(database)
    .await
}
