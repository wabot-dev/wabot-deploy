//! One organisation's console. See `docs/architecture-rules.md`.
//!
//! `/node`, `/updates` and `/organisations` are not registered on this
//! router, so they 404 here by construction rather than by a redirect
//! somebody has to remember to write.

use std::sync::Arc;

use hypertext::prelude::*;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use super::auth::{
    back_with_error, field, read_form, see_other, signed_in, JoinPage, PageQuery, SessionMiddleware,
};
use super::ConsoleState;
use crate::accounts::roles::{NodeRole, ProjectRole};
use crate::accounts::{self, invitations};
use crate::policies::access;
use crate::views::layouts::layout;
use crate::views::layouts::shell::{Area, Frame};
use crate::views::texts::language::t;

#[injectable]
pub struct TenantPeoplePages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl TenantPeoplePages {
    /// Its own, because `nodes.rs` is not on this router.
    #[view("/settings")]
    #[middleware(SessionMiddleware)]
    async fn settings(&self) -> UiResult<ViewOutcome> {
        match signed_in(&self.auth)
            .is_some_and(|account| crate::authz::tenant_user::administers(&account))
        {
            true => Ok(Redirect::found("/people").into()),
            false => Ok(Redirect::found("/").into()),
        }
    }

    #[view("/people")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::tenant_user::administers(&account) {
            return Ok(Redirect::found("/").into());
        }

        let people = accounts::of_scope(&self.state.database, account.tenant_id.as_deref()).await?;
        let all = invitations::all(&self.state.database).await?;
        let projects = access::projects_for(&self.state.database, &account).await?;
        let live: Vec<&crate::accounts::invitations::Invitation> = all
            .iter()
            .filter(|invitation| invitation.tenant_id == account.tenant_id)
            .filter(|invitation| invitation.live(crate::clock::now_ms()))
            .collect();

        layout::head("People");
        let frame = Frame::new(&account, Area::Settings, &projects, None, "/people");
        let page = crate::views::pages::tenant_user_people_page::People {
            people: &people,
            live: &live,
            projects: &projects,
            account: &account,
            query_error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::tenant_user_people_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// The one page a stranger may open, on the organisation's own
    /// address — which is the address every invitation into it carries.
    #[view("/join/:token")]
    #[middleware(SessionMiddleware)]
    async fn join(&self, query: JoinPage) -> UiResult<ViewOutcome> {
        if signed_in(&self.auth).is_some() {
            return Ok(Redirect::found("/?error=sign+out+first+to+accept+an+invitation").into());
        }

        let token = query.token.clone();
        let invitation = invitations::look_up(&self.state.database, &token).await?;

        layout::head("Join");
        let heading = match invitation
            .as_ref()
            .and_then(|found| found.scope_name.as_deref())
        {
            Some(name) => name.to_string(),
            None => t("Join this organisation").to_string(),
        };
        let page = crate::views::pages::guest_join_page::Join {
            heading: &heading,
            token: &token,
            invitation: invitation.as_ref(),
            error: query.error.as_deref(),
        };
        Ok(crate::views::pages::guest_join_page::render(&page)
            .into_view()
            .into())
    }
}

#[injectable]
pub struct TenantPeopleApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl TenantPeopleApi {
    #[post("/people/invite")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn invite(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let asked = crate::dtos::requests::invite_person::InvitePerson {
            role: NodeRole::parse(field(&form, "node_role")),
            project: field(&form, "project").to_string(),
            project_role: ProjectRole::parse(field(&form, "project_role")),
        };

        let invited = match crate::usecases::tenant_user_invite_person::run(
            &self.state.database,
            &account,
            &asked,
        )
        .await
        {
            Ok(invited) => invited,
            Err(reason) => return Ok(back_with_error("/people", &reason)),
        };

        // On the organisation's own name: the node's would sign them up
        // and bounce them on their first click.
        let link = format!(
            "{}/join/{}",
            self.state.base_url_for(account.tenant_id.as_deref()).await,
            invited.token
        );
        let nonce = self.state.reveals.stash(link);
        Ok(see_other(&format!(
            "/people/shown?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("nonce", &nonce)
                .finish()
        )))
    }

    #[post("/people/:account/role")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_role(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/people"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let role = match field(&form, "role") {
            "admin" => NodeRole::Admin,
            _ => NodeRole::Member,
        };

        match crate::usecases::tenant_user_set_person_role::run(
            &self.state.database,
            &account,
            id,
            role,
        )
        .await
        {
            Ok(()) => Ok(see_other("/people")),
            Err(reason) => Ok(back_with_error("/people", &reason)),
        }
    }

    #[post("/people/:account/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn remove(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/people"));
        };

        match crate::usecases::tenant_user_remove_person::run(&self.state.database, &account, id)
            .await
        {
            Ok(()) => Ok(see_other("/people")),
            Err(reason) => Ok(back_with_error("/people", &reason)),
        }
    }

    #[post("/people/invitations/:invitation/revoke")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revoke(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(2).cloned() else {
            return Ok(see_other("/people"));
        };

        match crate::usecases::tenant_user_revoke_invitation::run(
            &self.state.database,
            &account,
            id,
        )
        .await
        {
            Ok(()) => Ok(see_other("/people")),
            Err(reason) => Ok(back_with_error("/people", &reason)),
        }
    }

    #[post("/join/:token")]
    #[raw]
    async fn join(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some(token) = super::auth::segments(&path).get(1).map(|id| id.to_string()) else {
            return Ok(see_other("/sign-in"));
        };
        // Which organisation this address answers for, decided by the
        // `Host` before anything was read. An invitation minted in
        // another one is not a door into this.
        let Some(scope) = request
            .extensions()
            .get::<crate::edge::Tenancy>()
            .map(|tenancy| tenancy.0.clone())
        else {
            return Ok(see_other("/sign-in"));
        };
        let here = format!("/join/{token}");

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let account = match crate::usecases::guest_join_as_tenant_user::run(
            &self.state.database,
            &scope,
            &super::auth::joining(&token, &form),
        )
        .await
        {
            Ok(account) => account,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        match crate::accounts::sessions::create(&self.state.database, &account).await {
            Ok(token) => Ok(super::auth::see_other_with_cookie(
                "/",
                crate::accounts::sessions::set_cookie(&token),
            )),
            Err(error) => {
                tracing::error!(%error, "could not start a session after joining");
                Ok(see_other("/sign-in"))
            }
        }
    }
}
