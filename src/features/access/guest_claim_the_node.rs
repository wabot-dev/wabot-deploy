use wabot::sqlite::SqliteDatabase;

use crate::features::access::claim_the_node_request::ClaimTheNodeRequest;
use crate::features::access::signed_in_response::SignedInResponse;
use crate::features::access::welcome_command::Welcome;
use crate::infra::accounts::{self, ceremonies};

/// The first account on a node, made against the token `install`
/// printed. There is nobody to authorise it because there is nobody yet,
/// and the token is the second factor that trusts its browser.
pub async fn run(
    database: &SqliteDatabase,
    asked: &ClaimTheNodeRequest,
) -> Result<SignedInResponse, String> {
    let made = match asked.passkey.given() {
        true => {
            let rp = ceremonies::relying_party(&asked.browser.host)
                .ok_or(ceremonies::NO_PASSKEYS_HERE)?;
            let made = ceremonies::finish_registration(database, &rp, &asked.passkey, None).await?;
            Some((rp, made))
        }
        false => None,
    };

    let account = accounts::create_admin(
        database,
        &asked.setup_token,
        &asked.username,
        made.as_ref().map(|(_, made)| made.user_handle.as_str()),
    )
    .await
    .map_err(|error| error.to_string())?;

    Welcome {
        account: &account,
        passkey: made.as_ref().map(|(rp, made)| (rp, made)),
        browser: &asked.browser,
        trust: made.is_some(),
        news: false,
    }
    .run(database)
    .await
}
