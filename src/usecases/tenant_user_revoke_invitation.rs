use wabot::sqlite::SqliteDatabase;

use crate::accounts::{invitations, Account};
use crate::authz::tenant_user;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    invitation: &str,
) -> Result<(), String> {
    if !tenant_user::administers(account) {
        return Err("only an administrator of this organisation can do that".into());
    }

    let theirs = invitations::all(database)
        .await
        .map_err(|error| error.to_string())?;
    if !theirs
        .iter()
        .any(|found| found.id == invitation && found.tenant_id == account.tenant_id)
    {
        return Err("no such invitation here".into());
    }

    invitations::revoke(database, invitation)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::roles::{NodeRole, ProjectRole};
    use crate::dtos::requests::invite_person::InvitePerson;
    use crate::usecases::fixtures::{organisation, Organisation};
    use crate::usecases::tenant_user_invite_person;

    async fn invited_by(here: &Organisation, account: &Account) -> String {
        tenant_user_invite_person::run(
            &here.database,
            account,
            &InvitePerson {
                role: NodeRole::Member,
                project: String::new(),
                project_role: ProjectRole::Viewer,
            },
        )
        .await
        .expect("invited");

        invitations::all(&here.database)
            .await
            .expect("invitations")
            .into_iter()
            .find(|found| found.tenant_id == account.tenant_id)
            .expect("theirs")
            .id
    }

    #[tokio::test]
    async fn an_administrator_revokes_one_of_their_own() {
        let here = organisation().await;
        let invitation = invited_by(&here, &here.admin).await;

        run(&here.database, &here.admin, &invitation)
            .await
            .expect("revoked");

        assert!(invitations::all(&here.database)
            .await
            .expect("invitations")
            .iter()
            .all(|found| found.id != invitation));
    }

    /// An invitation is a way into an organisation, so revoking one
    /// across the boundary would let somebody shut another organisation
    /// out of its own door.
    #[tokio::test]
    async fn nobody_revokes_an_invitation_of_another_organisation() {
        let here = organisation().await;
        let theirs = invited_by(&here, &here.rival).await;

        let refused = run(&here.database, &here.admin, &theirs)
            .await
            .expect_err("refused");
        assert!(refused.contains("no such invitation here"), "{refused}");

        assert!(invitations::all(&here.database)
            .await
            .expect("invitations")
            .iter()
            .any(|found| found.id == theirs));
    }

    #[tokio::test]
    async fn a_member_revokes_nothing() {
        let here = organisation().await;
        let invitation = invited_by(&here, &here.admin).await;

        assert!(run(&here.database, &here.member, &invitation)
            .await
            .is_err());
    }
}
