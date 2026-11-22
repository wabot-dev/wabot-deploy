use wabot::sqlite::SqliteDatabase;

use crate::features::access::signed_in_response::SignedInResponse;
use crate::features::access::welcome_command::Welcome;
use crate::features::people::join_request::JoinRequest;
use crate::infra::accounts::{ceremonies, invitations};

pub async fn run(
    database: &SqliteDatabase,
    asked: &JoinRequest,
) -> Result<SignedInResponse, String> {
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

    let made = match asked.passkey.given() {
        true => {
            let rp = ceremonies::relying_party(&asked.browser.host)
                .ok_or(ceremonies::NO_PASSKEYS_HERE)?;
            let made = ceremonies::finish_registration(database, &rp, &asked.passkey, None).await?;
            Some((rp, made))
        }
        false => None,
    };
    let account = invitations::accept(
        database,
        &asked.token,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::roles::{NodeRole, ProjectRole};
    use crate::features::fixtures::organisation;
    use crate::features::people::invite_person_request::InvitePersonRequest;
    use crate::features::people::{node_user_invite_person, tenant_user_invite_person};

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
            &InvitePersonRequest {
                role: NodeRole::Member,
                project: String::new(),
                project_role: ProjectRole::Viewer,
            },
        )
        .await
        .expect("invited");

        let refused = run(
            &here.database,
            &JoinRequest {
                token: invited.token.clone(),
                username: "newcomer".into(),
                ..Default::default()
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
            &InvitePersonRequest {
                role: NodeRole::Member,
                project: String::new(),
                project_role: ProjectRole::Viewer,
            },
        )
        .await
        .expect("invited");

        let born = run(
            &here.database,
            &JoinRequest {
                token: invited.token.clone(),
                username: "newcomer".into(),
                ..Default::default()
            },
        )
        .await
        .expect("joined");
        let born = crate::infra::accounts::sessions::lookup(&here.database, &born.session)
            .await
            .expect("read")
            .expect("signed in");
        assert_eq!(born.tenant_id, None);
    }
}
