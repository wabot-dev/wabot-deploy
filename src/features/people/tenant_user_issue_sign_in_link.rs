use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::features::people::issue_sign_in_link_command::IssueSignInLink;
use crate::features::people::issue_sign_in_link_request::IssueSignInLinkRequest;
use crate::features::people::sign_in_link_response::SignInLinkResponse;
use crate::features::people::tenant_user_sign_out_everywhere::reachable;

/// For somebody they could remove: their organisation's people, and never
/// themselves.
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
