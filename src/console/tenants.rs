//! Organisations, and what each of them may take.
//!
//! ## The node's operator only
//!
//! Making an organisation, giving it a domain and setting its ceiling
//! are decisions about the *machine* — how it is shared out — so they
//! sit beside the node's own settings and ask `is_admin`, which a
//! tenant's administrator is not. What a tenant's administrator manages
//! is what is inside their frontier: their projects and their people.
//!
//! ## The domain is the frontier
//!
//! A tenant's console answers on its own hostname, and `edge::dispatch`
//! stamps the scope on the request before any handler runs — so the
//! narrowing is the route rather than something thirty queries have to
//! remember. Setting the domain here is therefore what brings a tenant's
//! console into existence at all; without one it has projects and
//! nowhere to administer them from.

use std::sync::Arc;

use hypertext::prelude::*;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use super::auth::{
    back_with_error, field, read_form, see_other, signed_in, PageQuery, SessionMiddleware,
};
use super::ConsoleState;
use crate::policies::access;
use crate::repositories::tenants;
use crate::views::layouts::layout;
use crate::views::layouts::shell::{Area, Frame};

#[injectable]
pub struct TenantPages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl TenantPages {
    /// Every organisation on this node, what it answers to and what it
    /// may take.
    #[view("/organisations")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(Redirect::found("/").into());
        }

        // The head, or the page renders as a document with no
        // stylesheet — nav, content and all, in Times New Roman. It is
        // what every other view does first and the one thing a new page
        // has nothing reminding it of.
        layout::head("Organisations");

        let rows = tenants::all(&self.state.database).await?;
        let mut cards = Vec::new();
        for tenant in &rows {
            let held = access::of_tenant(&self.state.database, &tenant.id)
                .await
                .unwrap_or_default();
            cards.push((tenant.clone(), held));
        }

        let frame = Frame::new(&account, Area::Settings, &[], None, "/organisations");
        let page = crate::views::pages::node_user_organisations_page::Organisations {
            cards: &cards,
            query_error: query.error.as_deref(),
            query_invited: query.invited.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_organisations_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}

#[injectable]
pub struct TenantApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl TenantApi {
    #[post("/organisations")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(see_other("/"));
        }
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
            Ok(_) => Ok(see_other("/organisations")),
            Err(reason) => Ok(back_with_error("/organisations", &reason)),
        }
    }

    #[post("/organisations/:tenant/domain")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_domain(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
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
            Ok(()) => Ok(see_other("/organisations")),
            Err(reason) => Ok(back_with_error("/organisations", &reason)),
        }
    }

    #[post("/organisations/:tenant/quota")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_quota(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
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
            Ok(()) => Ok(see_other("/organisations")),
            Err(reason) => Ok(back_with_error("/organisations", &reason)),
        }
    }

    #[post("/organisations/:tenant/invite")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn invite(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let asked =
            crate::dtos::requests::invite_person_into_organisation::InvitePersonIntoOrganisation {
                organisation: id.to_string(),
                role: crate::accounts::roles::NodeRole::parse(field(&form, "node_role")),
                project: field(&form, "project").to_string(),
                project_role: crate::accounts::roles::ProjectRole::parse(field(
                    &form,
                    "project_role",
                )),
            };

        let invited = match crate::usecases::node_user_invite_person_into_organisation::run(
            &self.state.database,
            &account,
            &asked,
        )
        .await
        {
            Ok(invited) => invited,
            Err(reason) => return Ok(back_with_error("/organisations", &reason)),
        };

        // On the organisation's own name: the node's would sign them up
        // and bounce them on their first click.
        let link = format!(
            "{}/join/{}",
            self.state.base_url_for(Some(&invited.organisation)).await,
            invited.token
        );
        Ok(see_other(&format!(
            "/organisations?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("invited", &link)
                .finish()
        )))
    }

    #[post("/organisations/:tenant/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };

        match crate::usecases::node_user_delete_organisation::run(
            &self.state.database,
            &self.state.deployer,
            &account,
            id,
        )
        .await
        {
            Ok(()) => Ok(see_other("/organisations")),
            Err(reason) => Ok(back_with_error("/organisations", &reason)),
        }
    }
}
