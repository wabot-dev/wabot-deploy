use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::roles::NodeRole;
use crate::features::organisations::invite_owner_request::InviteOwnerRequest;
use crate::features::organisations::person_invited_into_organisation_response::PersonInvitedIntoOrganisationResponse;
use crate::infra::accounts::invitations;
use crate::infra::repositories::tenants;

/// The only way into an organisation, since one with nobody in it cannot
/// invite anybody either.
///
/// **An administrator, always.** There is no role to choose: a member
/// invited by the node's operator would reach one organisation and no
/// console that admits they exist, because it is an administrator who
/// makes the first project. And no project either — the node's operator
/// deciding who sees what inside somebody else's frontier is the thing
/// this use case exists to stop being possible.
pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &InviteOwnerRequest,
) -> Result<PersonInvitedIntoOrganisationResponse, String> {
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let Some(tenant) = tenants::find(database, &asked.organisation)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such organisation".into());
    };

    let token =
        invitations::create_into(database, account, NodeRole::Admin, None, Some(&tenant.id))
            .await
            .map_err(|error| error.to_string())?;

    Ok(PersonInvitedIntoOrganisationResponse {
        organisation: tenant.id,
        token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::fixtures::organisation;

    fn asked(organisation: &str) -> InviteOwnerRequest {
        InviteOwnerRequest {
            organisation: organisation.to_string(),
        }
    }

    #[tokio::test]
    async fn the_link_lands_in_the_organisation_it_names() {
        let here = organisation().await;
        let acme = here.admin.tenant_id.clone().expect("scoped");

        let invited = run(&here.database, &here.operator, &asked(&acme))
            .await
            .expect("invited");
        let found = invitations::look_up(&here.database, &invited.token)
            .await
            .expect("query")
            .expect("live");
        assert_eq!(found.tenant_id.as_deref(), Some(acme.as_str()));
    }

    /// **Whoever it lets in can run the place.** A member would reach
    /// one organisation and no console that admits they exist: it is an
    /// administrator who makes the first project, and this is the link
    /// that hands the organisation over.
    #[tokio::test]
    async fn whoever_takes_it_administers_the_organisation() {
        let here = organisation().await;
        let acme = here.admin.tenant_id.clone().expect("scoped");

        let invited = run(&here.database, &here.operator, &asked(&acme))
            .await
            .expect("invited");
        let found = invitations::look_up(&here.database, &invited.token)
            .await
            .expect("query")
            .expect("live");
        assert_eq!(found.node_role, NodeRole::Admin);
        assert!(
            found.project_id.is_none(),
            "it carries a project: {found:?}"
        );
    }

    #[tokio::test]
    async fn an_organisation_that_is_not_there_is_refused() {
        let here = organisation().await;
        let refused = run(&here.database, &here.operator, &asked("tnt-nothing"))
            .await
            .expect_err("refused");
        assert!(refused.contains("no such organisation"), "{refused}");
    }

    /// An organisation's own administrator is not the machine's, so
    /// handing out another organisation is not theirs to do.
    #[tokio::test]
    async fn an_organisations_administrator_cannot_run_it() {
        let here = organisation().await;
        let acme = here.admin.tenant_id.clone().expect("scoped");
        assert!(run(&here.database, &here.admin, &asked(&acme))
            .await
            .is_err());
    }
}
