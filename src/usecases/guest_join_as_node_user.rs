use wabot::sqlite::SqliteDatabase;

use crate::accounts::{invitations, Account};
use crate::dtos::requests::join::Join;

pub async fn run(database: &SqliteDatabase, asked: &Join) -> Result<Account, String> {
    let Some(invitation) = invitations::look_up(database, &asked.token)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err(
            "that invitation is not valid — it may have been used already, or expired".into(),
        );
    };

    // The door decides nothing about who they become; the invitation
    // does. What the door has to be is the one their account will work
    // at — a session cookie is host-only, so an organisation's person
    // signed in here would hold one their own console never receives.
    if invitation.tenant_id.is_some() {
        return Err(match &invitation.scope_name {
            Some(name) => format!(
                "that invitation is for {name} — open it on that organisation's own address"
            ),
            None => "that invitation is for an organisation, not for this node".into(),
        });
    }

    invitations::accept(database, &asked.token, &asked.username, &asked.password)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::roles::{NodeRole, ProjectRole};
    use crate::dtos::requests::invite_person::InvitePerson;
    use crate::usecases::fixtures::organisation;
    use crate::usecases::{node_user_invite_person, tenant_user_invite_person};

    /// The link an organisation's invitation carries points at that
    /// organisation's own address. Before there was a door there it
    /// answered 404; this is the other half — the node's door does not
    /// quietly take it either.
    #[tokio::test]
    async fn an_organisations_invitation_is_refused_here() {
        let here = organisation().await;
        let invited = tenant_user_invite_person::run(
            &here.database,
            &here.admin,
            &InvitePerson {
                role: NodeRole::Member,
                project: String::new(),
                project_role: ProjectRole::Viewer,
            },
        )
        .await
        .expect("invited");

        let refused = run(
            &here.database,
            &Join {
                token: invited.token.clone(),
                username: "newcomer".into(),
                password: "a long passphrase here".into(),
            },
        )
        .await
        .expect_err("refused");
        assert!(refused.contains("Acme"), "{refused}");
    }

    #[tokio::test]
    async fn the_machines_own_invitation_makes_somebody_of_the_machine() {
        let here = organisation().await;
        let invited = node_user_invite_person::run(
            &here.database,
            &here.operator,
            &InvitePerson {
                role: NodeRole::Member,
                project: String::new(),
                project_role: ProjectRole::Viewer,
            },
        )
        .await
        .expect("invited");

        let born = run(
            &here.database,
            &Join {
                token: invited.token.clone(),
                username: "newcomer".into(),
                password: "a long passphrase here".into(),
            },
        )
        .await
        .expect("joined");
        assert_eq!(born.tenant_id, None);
    }
}
