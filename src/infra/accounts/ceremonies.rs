//! Between the framework's WebAuthn and this node's tables: a challenge
//! issued for a page, answered by the browser, verified, and spent.

use serde_json::Value;
use wabot::sqlite::SqliteDatabase;
use wabot_addon_auth_passkey::{
    creation_options, request_options, to_base64url, verify_authentication, verify_registration,
    AuthenticationResponse, PasskeyError, Registered, RegistrationResponse, RelyingParty, User,
    UserVerification,
};

use super::challenges::{self, Purpose};
use super::passkeys::{self, Passkey};
use super::{sessions, AccountResult};
use crate::domain::entities::session::Session;
use crate::domain::values::browser::Ceremony;
use crate::infra::clock::now_ms;

const NAME: &str = "wabot-deploy";

pub const EXPIRED: &str = "that passkey prompt expired — try again";
pub const UNKNOWN: &str = "that passkey is not one this console knows";
pub const ASK_AGAIN: &str =
    "this needs your passkey again — anything that makes a way in asks for it after five minutes";
pub const NO_PASSKEYS_HERE: &str = "passkeys need this console opened by its name, not its address";

/// The relying party a host is, or none when it is an address.
pub fn relying_party(host: &str) -> Option<RelyingParty> {
    RelyingParty::for_host(host, NAME).ok()
}

/// What a page renders for a ceremony: the challenge it will be answered
/// against and the options the browser is handed.
#[derive(Debug, Clone)]
pub struct Begun {
    pub challenge: String,
    pub options: Value,
}

/// Options for making a passkey: for an account, or for one about to
/// exist, whose id is minted now because it is the passkey's user handle.
pub async fn begin_registration(
    database: &SqliteDatabase,
    rp: &RelyingParty,
    account: Option<(&str, &str)>,
) -> AccountResult<Begun> {
    let (account_id, handle, name) = match account {
        Some((id, name)) => (Some(id), id.to_string(), name.to_string()),
        None => (None, super::new_id(), String::new()),
    };
    let exclude = match account_id {
        Some(id) => passkeys::ids_for(database, id, &rp.id).await?,
        None => Vec::new(),
    };
    let challenge =
        challenges::issue(database, Purpose::Register, account_id, Some(&handle)).await?;
    let user = User {
        handle: handle.as_bytes(),
        name: &name,
        display_name: &name,
    };
    Ok(Begun {
        options: creation_options(rp, &user, &challenge, &exclude),
        challenge,
    })
}

/// Options for answering with a passkey: any of this host's when nobody
/// is named, which is signing in, or only the account's.
pub async fn begin_assertion(
    database: &SqliteDatabase,
    rp: &RelyingParty,
    purpose: Purpose,
    account_id: Option<&str>,
) -> AccountResult<Begun> {
    let allow = match account_id {
        Some(id) => passkeys::ids_for(database, id, &rp.id).await?,
        None => Vec::new(),
    };
    let challenge = challenges::issue(database, purpose, account_id, None).await?;
    Ok(Begun {
        options: request_options(rp, &challenge, &allow),
        challenge,
    })
}

#[derive(Debug, Clone)]
pub struct Made {
    pub user_handle: String,
    pub registered: Registered,
}

pub async fn finish_registration(
    database: &SqliteDatabase,
    rp: &RelyingParty,
    ceremony: &Ceremony,
    account_id: Option<&str>,
) -> Result<Made, String> {
    let found = challenges::spend(database, &ceremony.challenge, Purpose::Register)
        .await
        .map_err(|error| error.to_string())?
        .filter(|found| found.account_id.as_deref() == account_id)
        .ok_or(EXPIRED)?;
    let response = RegistrationResponse::from_json(&ceremony.credential).map_err(reason)?;
    let registered = verify_registration(rp, &found.value, &response, UserVerification::Required)
        .map_err(reason)?;
    let id = to_base64url(&registered.credential_id);
    if passkeys::find(database, &id)
        .await
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("that passkey is already on an account here".into());
    }
    Ok(Made {
        user_handle: found.user_handle.unwrap_or_default(),
        registered,
    })
}

pub async fn finish_assertion(
    database: &SqliteDatabase,
    rp: &RelyingParty,
    ceremony: &Ceremony,
    purpose: Purpose,
    account_id: Option<&str>,
) -> Result<Passkey, String> {
    let found = challenges::spend(database, &ceremony.challenge, purpose)
        .await
        .map_err(|error| error.to_string())?
        .filter(|found| found.account_id.is_none() || found.account_id.as_deref() == account_id)
        .ok_or(EXPIRED)?;
    let response = AuthenticationResponse::from_json(&ceremony.credential).map_err(reason)?;
    let id = to_base64url(&response.credential_id().map_err(reason)?);
    let passkey = passkeys::find(database, &id)
        .await
        .map_err(|error| error.to_string())?
        .filter(|passkey| passkey.rp_id == rp.id)
        .ok_or(UNKNOWN)?;
    if account_id.is_some_and(|expected| expected != passkey.account_id) {
        return Err("that passkey belongs to somebody else".into());
    }
    if let Some(handle) = response.user_handle().map_err(reason)? {
        if handle != passkey.account_id.as_bytes() {
            return Err(UNKNOWN.into());
        }
    }
    let answered = verify_authentication(
        rp,
        &found.value,
        &response,
        &passkey.public_key,
        passkey.sign_count,
        UserVerification::Required,
    )
    .map_err(reason)?;
    passkeys::used(
        database,
        &passkey.id,
        answered.sign_count,
        answered.backed_up,
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(passkey)
}

/// Whoever holds this session proved who they are in the last five
/// minutes, or proves it now with the ceremony.
pub async fn asked_again(
    database: &SqliteDatabase,
    host: &str,
    session: &Session,
    account_id: &str,
    ceremony: &Ceremony,
) -> Result<(), String> {
    if session.fresh(now_ms()) {
        return Ok(());
    }
    if !ceremony.given() {
        return Err(ASK_AGAIN.into());
    }
    let rp = relying_party(host).ok_or(NO_PASSKEYS_HERE)?;
    finish_assertion(database, &rp, ceremony, Purpose::Verify, Some(account_id)).await?;
    sessions::mark_verified(database, &session.token_hash)
        .await
        .map_err(|error| error.to_string())
}

fn reason(error: PasskeyError) -> String {
    match error {
        PasskeyError::UserNotVerified => {
            "the device did not verify it was you — passkeys here need a fingerprint, a face or a \
             PIN"
            .into()
        }
        PasskeyError::Origin(_) | PasskeyError::RelyingParty | PasskeyError::CrossOrigin => {
            "that passkey answered for another site".into()
        }
        PasskeyError::CounterWentBack => {
            "that passkey's counter went backwards, which is what a copied key does — remove it \
             from Your account and make another"
                .into()
        }
        other => format!("the passkey could not be verified: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wabot_addon_auth_passkey::testing::SoftAuthenticator;

    async fn someone() -> (SqliteDatabase, crate::domain::entities::account::Account) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let token = super::super::issue_setup_token(&database)
            .await
            .expect("token");
        let account = super::super::create_admin(&database, &token, "jorge", None)
            .await
            .expect("created");
        (database, account)
    }

    async fn enrolled(
        database: &SqliteDatabase,
        rp: &RelyingParty,
        account_id: &str,
    ) -> SoftAuthenticator {
        let device = SoftAuthenticator::new();
        let begun = begin_registration(database, rp, Some((account_id, "jorge")))
            .await
            .expect("begun");
        let made = finish_registration(
            database,
            rp,
            &Ceremony {
                credential: device.register(rp, &begun.challenge, account_id.as_bytes()),
                challenge: begun.challenge,
            },
            Some(account_id),
        )
        .await
        .expect("made");
        passkeys::insert(database, account_id, &rp.id, &made.registered, "test")
            .await
            .expect("stored");
        device
    }

    #[tokio::test]
    async fn a_passkey_made_here_signs_in_here_and_nowhere_else() {
        let (database, account) = someone().await;
        let rp = relying_party("node.example.com").expect("a name");
        let device = enrolled(&database, &rp, &account.id).await;

        let begun = begin_assertion(&database, &rp, Purpose::SignIn, None)
            .await
            .expect("begun");
        let passkey = finish_assertion(
            &database,
            &rp,
            &Ceremony {
                credential: device.authenticate(&rp, &begun.challenge),
                challenge: begun.challenge.clone(),
            },
            Purpose::SignIn,
            None,
        )
        .await
        .expect("signed in");
        assert_eq!(passkey.account_id, account.id);

        let elsewhere = relying_party("other.example.com").expect("a name");
        let begun = begin_assertion(&database, &elsewhere, Purpose::SignIn, None)
            .await
            .expect("begun");
        let refused = finish_assertion(
            &database,
            &elsewhere,
            &Ceremony {
                credential: device.authenticate(&elsewhere, &begun.challenge),
                challenge: begun.challenge,
            },
            Purpose::SignIn,
            None,
        )
        .await;
        assert_eq!(refused.err().as_deref(), Some(UNKNOWN));
    }

    /// A page on a subdomain of the console's host may ask for the
    /// console's passkeys; its own origin is signed into the answer.
    #[tokio::test]
    async fn an_answer_given_to_another_origin_is_refused() {
        let (database, account) = someone().await;
        let rp = relying_party("node.example.com").expect("a name");
        let device = enrolled(&database, &rp, &account.id).await;

        let begun = begin_assertion(&database, &rp, Purpose::SignIn, None)
            .await
            .expect("begun");
        let refused = finish_assertion(
            &database,
            &rp,
            &Ceremony {
                credential: device.authenticate_at(
                    &rp,
                    "https://shop.node.example.com",
                    &begun.challenge,
                ),
                challenge: begun.challenge,
            },
            Purpose::SignIn,
            None,
        )
        .await;
        assert!(refused.is_err());
    }

    #[tokio::test]
    async fn a_challenge_answers_once() {
        let (database, account) = someone().await;
        let rp = relying_party("node.example.com").expect("a name");
        let device = enrolled(&database, &rp, &account.id).await;
        let begun = begin_assertion(&database, &rp, Purpose::SignIn, None)
            .await
            .expect("begun");
        let ceremony = Ceremony {
            credential: device.authenticate(&rp, &begun.challenge),
            challenge: begun.challenge,
        };
        finish_assertion(&database, &rp, &ceremony, Purpose::SignIn, None)
            .await
            .expect("first");
        let again = finish_assertion(&database, &rp, &ceremony, Purpose::SignIn, None).await;
        assert_eq!(again.err().as_deref(), Some(EXPIRED));
    }

    #[tokio::test]
    async fn an_address_is_no_relying_party() {
        assert!(relying_party("127.0.0.1:9444").is_none());
        assert!(relying_party("localhost:9443").is_some());
    }
}
