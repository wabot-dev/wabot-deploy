//! The pages of one organisation's own people.
//!
//! `/node`, `/updates` and `/organisations` are not registered on this
//! router, so they 404 here by construction rather than by a redirect
//! somebody has to remember to write.

use std::sync::Arc;

use hypertext::prelude::*;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::features::people::texts::t;
use crate::infra::accounts::{self, invitations};
use crate::infra::repositories::access;
use crate::shell::auth::{signed_in, JoinPage, PageQuery, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Area, Frame};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct TenantPeopleUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl TenantPeopleUiController {
    /// Its own, because `nodes.rs` is not on this router.
    #[view("/settings")]
    #[middleware(SessionMiddleware)]
    async fn settings(&self) -> UiResult<ViewOutcome> {
        match signed_in(&self.auth)
            .is_some_and(|account| crate::domain::authz::tenant_user::administers(&account))
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
        if !crate::domain::authz::tenant_user::administers(&account) {
            return Ok(Redirect::found("/").into());
        }

        let people = accounts::of_scope(&self.state.database, account.tenant_id.as_deref()).await?;
        let all = invitations::all(&self.state.database).await?;
        let projects = access::projects_for(&self.state.database, &account).await?;
        let live: Vec<&crate::infra::accounts::invitations::Invitation> = all
            .iter()
            .filter(|invitation| invitation.tenant_id == account.tenant_id)
            .filter(|invitation| invitation.live(crate::infra::clock::now_ms()))
            .collect();

        layout::head("People");
        let frame = Frame::new(&account, Area::Settings, &projects, None, "/people");
        let page = crate::features::people::tenant_user_people_page::People {
            people: &people,
            live: &live,
            projects: &projects,
            account: &account,
            query_error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::people::tenant_user_people_page::render(&page)
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
        let page = crate::features::people::guest_join_page::Join {
            heading: &heading,
            token: &token,
            invitation: invitation.as_ref(),
            error: query.error.as_deref(),
        };
        Ok(crate::features::people::guest_join_page::render(&page)
            .into_view()
            .into())
    }
}
