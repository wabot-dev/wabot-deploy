//! What the organisations pages post to.
//!
//! ## Inside one, only who answers for it
//!
//! The organisations page used to carry an invitation form with a role
//! and a project selector, which made the operator an administrator
//! inside a frontier that is not theirs. What they decide is *who
//! answers for this organisation*; members, project membership and
//! everything else is the organisation's own, from its own console.
//!
//! The one reach that stays is removing an administrator, and it earns
//! itself: an organisation whose last one has gone cannot invite a
//! replacement, so without it the only way to hand one over would be to
//! delete it with its projects inside.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::features::organisations::node_user_organisation_settings_page::OrganisationTab;
use crate::infra::repositories::tenants;
use crate::shell::auth::{
    back_with_error, field, read_form, see_other, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct TenantRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

impl TenantRestController {
    /// The operator, or the response that sends whoever else away.
    /// Boxed, because a `Response` is 128 bytes and every handler here
    /// returns this on its happy path too — the compiler sizes a
    /// `Result` by its larger arm.
    fn operator(&self) -> Result<crate::domain::entities::account::Account, Box<Response>> {
        match signed_in(&self.auth) {
            None => Err(Box::new(see_other("/sign-in"))),
            Some(account) => {
                match crate::domain::authz::node_user::shares_the_machine_out(&account) {
                    true => Ok(account),
                    false => Err(Box::new(see_other("/"))),
                }
            }
        }
    }
}

#[rest_controller("/")]
impl TenantRestController {
    #[post("/organisations")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::features::organisations::node_user_create_organisation::run(
            &self.state.database,
            &account,
            field(&form, "name"),
        )
        .await
        {
            // Straight to the one just made: the operator's next move is
            // to give it a domain and hand it to somebody, and both are
            // on its own page.
            Ok(tenant) => Ok(see_other(&OrganisationTab::Domain.path(&tenant.id))),
            Err(reason) => Ok(back_with_error("/organisations", &reason)),
        }
    }

    #[post("/organisations/:tenant/domain")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_domain(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let path = request.uri().path().to_string();
        let Some(id) = crate::shell::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Domain.path(id);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::features::organisations::node_user_set_organisation_domain::run(
            &self.state.database,
            &self.state.config,
            &self.state.deployer,
            &account,
            id,
            field(&form, "domain"),
        )
        .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/organisations/:tenant/quota")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_quota(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let path = request.uri().path().to_string();
        let Some(id) = crate::shell::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Quota.path(id);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::features::organisations::node_user_set_organisation_quota::run(
            &self.state.database,
            &account,
            id,
            field(&form, "memory"),
            field(&form, "cpu"),
        )
        .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    /// Hand the organisation to somebody.
    #[post("/organisations/:tenant/owners")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn invite_owner(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let path = request.uri().path().to_string();
        let Some(id) = crate::shell::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Owner.path(id);

        let asked = crate::features::organisations::invite_owner_request::InviteOwnerRequest {
            organisation: id.to_string(),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&here, &reason.to_string()));
        }

        let invited = match crate::features::organisations::node_user_invite_owner::run(
            &self.state.database,
            &account,
            &asked,
        )
        .await
        {
            Ok(invited) => invited,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        // On the organisation's own name: the node's would sign them up
        // and bounce them on their first click.
        let link = format!(
            "{}/join/{}",
            self.state.base_url_for(Some(&invited.organisation)).await,
            invited.token
        );
        // A page of its own, and the nonce rather than the link: an
        // invitation is a secret, and it used to travel in the query
        // string where the address bar, the history and every refresh
        // could read it.
        let nonce = self.state.reveals.stash(link);
        Ok(see_other(&format!(
            "/organisations/{id}/shown?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("nonce", &nonce)
                .finish()
        )))
    }

    #[post("/organisations/:tenant/owners/:account/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn remove_owner(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let path = request.uri().path().to_string();
        let segments = crate::shell::auth::segments(&path);
        let (Some(tenant), Some(person)) = (segments.get(1), segments.get(3)) else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Owner.path(tenant);

        match crate::features::organisations::node_user_remove_owner::run(
            &self.state.database,
            &account,
            person,
        )
        .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/organisations/:tenant/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let path = request.uri().path().to_string();
        let Some(id) = crate::shell::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Danger.path(id);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        // **The name, typed.** The same confirmation a service asks for,
        // and this one takes more with it: every project, every person
        // and every copy running anywhere.
        let Some(tenant) = tenants::find(&self.state.database, id).await? else {
            return Ok(see_other("/organisations"));
        };
        if field(&form, "confirm") != tenant.slug {
            return Ok(back_with_error(
                &here,
                "type the organisation's slug to confirm",
            ));
        }

        match crate::features::organisations::node_user_delete_organisation::run(
            &self.state.database,
            &self.state.deployer,
            &account,
            id,
        )
        .await
        {
            Ok(()) => Ok(see_other("/organisations")),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::web::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    /// Deleting an organisation takes its projects, its people and the
    /// invitations into it, as the danger card says.
    ///
    /// **Shipped otherwise**: the delete refused while a project remained
    /// and otherwise failed on a foreign key, with the database's own
    /// words on the page. Found on a VM node.
    #[tokio::test]
    async fn deleting_an_organisation_takes_what_is_in_it() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let created = console
            .harness
            .post("/organisations")
            .header("cookie", &cookie)
            .form(&[("name", "Acme")])
            .send()
            .await;
        let id = created
            .header("location")
            .and_then(|to| to.split('/').nth(2))
            .expect("the organisation's page")
            .to_string();
        let tenant = crate::infra::repositories::tenants::find(&console.database, &id)
            .await
            .expect("read")
            .expect("there");
        let owner = crate::infra::accounts::create_in(
            &console.database,
            "ana",
            "a long passphrase here",
            crate::domain::values::roles::NodeRole::Admin,
            Some(&tenant.id),
        )
        .await
        .expect("an owner");
        crate::infra::repositories::projects::create_for(
            &console.database,
            "storefront",
            Some(&tenant.id),
        )
        .await
        .expect("a project");
        crate::infra::accounts::invitations::create_into(
            &console.database,
            &owner,
            crate::domain::values::roles::NodeRole::Member,
            None,
            Some(&tenant.id),
        )
        .await
        .expect("an invitation");

        let deleted = console
            .harness
            .post(&format!("/organisations/{id}/delete"))
            .header("cookie", &cookie)
            .form(&[("confirm", tenant.slug.as_str())])
            .send()
            .await;
        let location = deleted.header("location").unwrap_or_default();
        assert!(!location.contains("error="), "{location}");

        assert!(
            crate::infra::repositories::tenants::find(&console.database, &id)
                .await
                .expect("read")
                .is_none()
        );
        assert!(
            crate::infra::repositories::projects::find(&console.database, "storefront")
                .await
                .expect("read")
                .is_none(),
            "its project went with it"
        );
        assert!(
            crate::infra::accounts::all(&console.database)
                .await
                .expect("read")
                .iter()
                .all(|account| account.username != "ana"),
            "and its people"
        );
    }

    /// An organisation's name has to reach this node before it is saved.
    ///
    /// **Shipped otherwise**: nothing was asked, so a typo was stored,
    /// routed and shown as the organisation's console, which never
    /// answered on it. Found on a VM node with `nowhere.invalid`.
    #[tokio::test]
    async fn an_organisation_is_given_only_a_name_that_arrives_here() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let created = console
            .harness
            .post("/organisations")
            .header("cookie", &cookie)
            .form(&[("name", "Acme")])
            .send()
            .await;
        let id = created
            .header("location")
            .and_then(|to| to.split('/').nth(2))
            .expect("the organisation's page")
            .to_string();
        let save = |domain: &'static str| {
            let console = &console;
            let cookie = &cookie;
            let path = format!("/organisations/{id}/domain");
            async move {
                let response = console
                    .harness
                    .post(&path)
                    .header("cookie", cookie)
                    .form(&[("domain", domain)])
                    .send()
                    .await;
                response.assert_status(StatusCode::SEE_OTHER);
                response.header("location").unwrap_or_default().to_string()
            }
        };
        let domain = || async {
            crate::infra::repositories::tenants::find(&console.database, &id)
                .await
                .expect("read")
                .expect("there")
                .domain
        };

        let refused = save("nowhere.invalid").await;
        assert!(refused.contains("error="), "{refused}");
        assert!(refused.contains("does+not+resolve"), "{refused}");
        assert_eq!(domain().await, None, "a refused name is not stored");

        let saved = save("localhost").await;
        assert!(!saved.contains("error="), "{saved}");
        assert_eq!(domain().await.as_deref(), Some("localhost"));
    }
}
