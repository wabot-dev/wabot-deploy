use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::features::people::sign_out_everywhere_command::SignOutEverywhere;
use crate::features::people::sign_out_everywhere_request::SignOutEverywhereRequest;
use crate::infra::accounts;

/// Who the node's administrators may let in or shut out: the node's own
/// people, and the administrators of its organisations, but not
/// themselves.
pub(crate) async fn reachable(
    database: &SqliteDatabase,
    account: &Account,
    person: &str,
) -> Result<Account, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if person == account.id {
        return Err("that is you — your own way in is a device you trust".into());
    }
    accounts::find(database, person)
        .await
        .map_err(|error| error.to_string())?
        .filter(|found| found.tenant_id.is_none() || found.administers_scope())
        .ok_or_else(|| "no such person here".into())
}

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &SignOutEverywhereRequest,
) -> Result<(), String> {
    let person = reachable(database, account, &asked.person).await?;
    SignOutEverywhere {
        by: account,
        person: &person,
    }
    .run(database)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::fixtures::organisation;
    use crate::features::people::issue_sign_in_link_request::IssueSignInLinkRequest;
    use crate::features::people::{node_user_issue_sign_in_link, tenant_user_issue_sign_in_link};

    /// An organisation's administrator never issues one for the node's
    /// people, and the operator reaches an organisation only as far as
    /// its administrators.
    #[tokio::test]
    async fn a_link_is_never_issued_upward() {
        let here = organisation().await;
        let ask = |person: &str| IssueSignInLinkRequest {
            person: person.to_string(),
        };

        assert!(tenant_user_issue_sign_in_link::run(
            &here.database,
            &here.admin,
            &ask(&here.operator.id)
        )
        .await
        .is_err());
        assert!(node_user_issue_sign_in_link::run(
            &here.database,
            &here.operator,
            &ask(&here.member.id)
        )
        .await
        .is_err());
        let link =
            node_user_issue_sign_in_link::run(&here.database, &here.operator, &ask(&here.admin.id))
                .await
                .expect("the operator reaches an organisation's administrator");
        assert_eq!(link.tenant_id, here.admin.tenant_id);
        assert!(tenant_user_issue_sign_in_link::run(
            &here.database,
            &here.admin,
            &ask(&here.member.id)
        )
        .await
        .is_ok());
        assert!(node_user_issue_sign_in_link::run(
            &here.database,
            &here.operator,
            &ask(&here.operator.id)
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn signing_out_everywhere_ends_every_session_and_every_device() {
        let here = organisation().await;
        let (device, _) = accounts::devices::trust(
            &here.database,
            None,
            &here.member.id,
            "Safari on iPhone",
            None,
            None,
        )
        .await
        .expect("trusted");
        let session = accounts::sessions::start(&here.database, &here.member, Some(&device))
            .await
            .expect("signed in");

        crate::features::people::tenant_user_sign_out_everywhere::run(
            &here.database,
            &here.admin,
            &SignOutEverywhereRequest {
                person: here.member.id.clone(),
            },
        )
        .await
        .expect("signed out");

        assert!(accounts::sessions::lookup(&here.database, &session)
            .await
            .expect("read")
            .is_none());
        assert!(
            accounts::devices::of_account(&here.database, &here.member.id)
                .await
                .expect("read")
                .is_empty()
        );
    }
}
