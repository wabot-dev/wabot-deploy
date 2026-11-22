use wabot::sqlite::SqliteDatabase;

use crate::features::access::sign_in_request::SignInRequest;
use crate::features::access::signed_in_response::SignedInResponse;
use crate::features::access::welcome_command::Welcome;
use crate::infra::accounts;

/// A password, which only an account from before passkeys still has.
///
/// **The frontier comes from the `Host`, not from the form.** A username
/// belongs to a scope, so three people called `jorge` is the ordinary
/// case and a search over the whole table would find whichever came
/// first — see migration `0057`.
pub async fn run(
    database: &SqliteDatabase,
    asked: &SignInRequest,
) -> Result<SignedInResponse, String> {
    let Some(account) = accounts::authenticate(
        database,
        &asked.username,
        &asked.password,
        asked.scope.as_deref(),
    )
    .await
    .map_err(|error| error.to_string())?
    else {
        return Err("that username and password do not match".into());
    };

    Welcome {
        account: &account,
        passkey: None,
        browser: &asked.browser,
        trust: false,
        news: false,
    }
    .run(database)
    .await
}
