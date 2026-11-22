use wabot::sqlite::SqliteDatabase;

use crate::features::access::redeem_approval_request::RedeemApprovalRequest;
use crate::features::access::signed_in_response::SignedInResponse;
use crate::features::access::welcome_command::Welcome;
use crate::infra::accounts;
use crate::infra::accounts::requests::{self, Redeemed};

pub const CLOSED: &str = "that request is closed — sign in again";

/// The six digits a trusted device showed, typed on the device that asked.
pub async fn run(
    database: &SqliteDatabase,
    asked: &RedeemApprovalRequest,
) -> Result<SignedInResponse, String> {
    let pending = asked.pending.as_deref().ok_or(CLOSED)?;
    let request = match requests::redeem(database, pending, &asked.code)
        .await
        .map_err(|error| error.to_string())?
    {
        Redeemed::Accepted(request) => request,
        Redeemed::NotYet => {
            return Err(
                "nobody has approved it yet — choose It was me on a device you trust".into(),
            )
        }
        Redeemed::Wrong => return Err("that is not the code it showed".into()),
        Redeemed::Gone => return Err(CLOSED.into()),
    };
    let account = accounts::find(database, &request.account_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or(CLOSED)?;

    Welcome {
        account: &account,
        passkey: None,
        browser: &asked.browser,
        trust: asked.trust,
        news: true,
    }
    .run(database)
    .await
}
