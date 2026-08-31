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

use super::auth::{back_with_error, field, read_form, see_other, signed_in, SessionMiddleware};
use super::ConsoleState;
use crate::repositories::tenants;
use crate::views::pages::node_user_organisation_settings_page::OrganisationTab;

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
    fn operator(&self) -> Result<crate::accounts::Account, Box<Response>> {
        match signed_in(&self.auth) {
            None => Err(Box::new(see_other("/sign-in"))),
            Some(account) => match crate::authz::node_user::shares_the_machine_out(&account) {
                true => Ok(account),
                false => Err(Box::new(see_other("/"))),
            },
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

        match crate::usecases::node_user_create_organisation::run(
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
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Domain.path(id);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::usecases::node_user_set_organisation_domain::run(
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
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Quota.path(id);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::usecases::node_user_set_organisation_quota::run(
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
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Owner.path(id);

        let asked = crate::dtos::requests::invite_owner::InviteOwner {
            organisation: id.to_string(),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&here, &reason.to_string()));
        }

        let invited = match crate::usecases::node_user_invite_owner::run(
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
        let segments = super::auth::segments(&path);
        let (Some(tenant), Some(person)) = (segments.get(1), segments.get(3)) else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Owner.path(tenant);

        match crate::usecases::node_user_remove_owner::run(&self.state.database, &account, person)
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
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
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

        match crate::usecases::node_user_delete_organisation::run(
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
