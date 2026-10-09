//! What one organisation's people page posts to.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::domain::values::roles::{NodeRole, ProjectRole};
use crate::shell::auth::{
    back_with_error, field, read_form, see_other, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct TenantPeopleRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl TenantPeopleRestController {
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

        let asked = crate::features::people::invite_person_request::InvitePersonRequest {
            role: NodeRole::parse(field(&form, "node_role")),
            project: field(&form, "project").to_string(),
            project_role: ProjectRole::parse(field(&form, "project_role")),
        };

        let invited = match crate::features::people::tenant_user_invite_person::run(
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
        let Some(id) = crate::shell::auth::segments(&path).get(1).cloned() else {
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

        match crate::features::people::tenant_user_set_person_role::run(
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
        let Some(id) = crate::shell::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/people"));
        };

        match crate::features::people::tenant_user_remove_person::run(
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

    #[post("/people/invitations/:invitation/revoke")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revoke(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(id) = crate::shell::auth::segments(&path).get(2).cloned() else {
            return Ok(see_other("/people"));
        };

        match crate::features::people::tenant_user_revoke_invitation::run(
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
        let Some(token) = crate::shell::auth::segments(&path)
            .get(1)
            .map(|id| id.to_string())
        else {
            return Ok(see_other("/sign-in"));
        };
        // Which organisation this address answers for, decided by the
        // `Host` before anything was read. An invitation minted in
        // another one is not a door into this.
        let Some(scope) = request
            .extensions()
            .get::<crate::infra::edge::Tenancy>()
            .map(|tenancy| tenancy.0.clone())
        else {
            return Ok(see_other("/sign-in"));
        };
        let here = format!("/join/{token}");

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let account = match crate::features::people::guest_join_as_tenant_user::run(
            &self.state.database,
            &scope,
            &crate::features::people::people_rest_controller::joining(&token, &form),
        )
        .await
        {
            Ok(account) => account,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        match crate::infra::accounts::sessions::create(&self.state.database, &account).await {
            Ok(token) => Ok(crate::shell::auth::see_other_with_cookie(
                "/",
                crate::infra::accounts::sessions::set_cookie(&token),
            )),
            Err(error) => {
                tracing::error!(%error, "could not start a session after joining");
                Ok(see_other("/sign-in"))
            }
        }
    }
}
