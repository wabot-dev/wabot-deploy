use wabot::sqlite::SqliteDatabase;

use crate::features::access::signed_in_response::SignedInResponse;
use crate::features::access::welcome_command::Welcome;
use crate::features::people::join_request::JoinRequest;
use crate::infra::accounts::{ceremonies, invitations};

pub async fn run(
    database: &SqliteDatabase,
    scope: &str,
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

    if invitation.tenant_id.as_deref() != Some(scope) {
        return Err("that invitation is not for this organisation".into());
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

    async fn token_for(here: &crate::features::fixtures::Organisation, admin: bool) -> String {
        match admin {
            true => {
                tenant_user_invite_person::run(
                    &here.database,
                    &here.admin,
                    &InvitePersonRequest {
                        role: NodeRole::Member,
                        project: String::new(),
                        project_role: ProjectRole::Viewer,
                    },
                )
                .await
                .expect("invited")
                .token
            }
            false => {
                node_user_invite_person::run(
                    &here.database,
                    &here.operator,
                    &InvitePersonRequest {
                        role: NodeRole::Member,
                        project: String::new(),
                        project_role: ProjectRole::Viewer,
                    },
                )
                .await
                .expect("invited")
                .token
            }
        }
    }

    #[tokio::test]
    async fn the_invitation_says_which_organisation_they_are_born_into() {
        let here = organisation().await;
        let token = token_for(&here, true).await;

        let born = run(
            &here.database,
            here.admin.tenant_id.as_deref().expect("scoped"),
            &JoinRequest {
                token: token.clone(),
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
        assert_eq!(born.tenant_id, here.admin.tenant_id);
    }

    #[tokio::test]
    async fn the_machines_own_invitation_is_refused_here() {
        let here = organisation().await;
        let token = token_for(&here, false).await;

        let refused = run(
            &here.database,
            here.admin.tenant_id.as_deref().expect("scoped"),
            &JoinRequest {
                token: token.clone(),
                username: "newcomer".into(),
                ..Default::default()
            },
        )
        .await
        .expect_err("refused");
        assert!(refused.contains("not for this organisation"), "{refused}");
    }

    /// Two organisations on one machine: a link minted in one is not a
    /// door into the other, whichever address it is opened at.
    #[tokio::test]
    async fn another_organisations_invitation_is_refused_here() {
        let here = organisation().await;
        let token = token_for(&here, true).await;

        assert!(run(
            &here.database,
            here.rival.tenant_id.as_deref().expect("scoped"),
            &JoinRequest {
                token: token.clone(),
                username: "newcomer".into(),
                ..Default::default()
            },
        )
        .await
        .is_err());
    }
}
