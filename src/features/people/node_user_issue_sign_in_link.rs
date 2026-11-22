use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::people::issue_sign_in_link_command::IssueSignInLink;
use crate::features::people::issue_sign_in_link_request::IssueSignInLinkRequest;
use crate::features::people::node_user_sign_out_everywhere::reachable;
use crate::features::people::sign_in_link_response::SignInLinkResponse;

/// **Never upward.** The node's administrators issue one for the node's
/// people and for an organisation's administrators, never for themselves:
/// their own way in is a device they trust.
pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &IssueSignInLinkRequest,
) -> Result<SignInLinkResponse, String> {
    let person = reachable(database, account, &asked.person).await?;
    IssueSignInLink {
        by: account,
        person: &person,
    }
    .run(database)
    .await
}
