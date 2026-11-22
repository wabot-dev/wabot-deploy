//! Who is on this node, and the one page a stranger may open.

use std::sync::Arc;

use hypertext::prelude::*;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::infra::accounts::{self, invitations};
use crate::infra::repositories::access;

use crate::shell::auth::{signed_in, JoinPage, PageQuery, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Frame, Section};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct PeopleUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
    asked: Arc<crate::shell::auth::Asked>,
}

#[ui_controller("/", app)]
impl PeopleUiController {
    /// Everybody on the node, and every invitation.
    ///
    /// Administrators only. Who else is here, and who has a live link
    /// to become somebody here, is the node's own business.
    #[view("/people")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        // **This machine's own operators**, and nobody else's people.
        // An organisation's are managed from its own console, by a
        // different controller — see `console::tenant`. This page knows
        // nothing about organisations, which is what lets that whole
        // half be deleted without touching it.
        let people = accounts::of_scope(&self.state.database, None).await?;

        let invitations = invitations::all(&self.state.database).await?;
        let projects = access::projects_for(&self.state.database, &account).await?;
        let now = crate::shell::state::now_ms();

        let frame = Frame::load(&self.state.database, &account, Section::People, "/people").await;
        layout::head("People");

        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let verify =
            crate::shell::auth::verify_prompt(&self.state, &self.auth, &self.asked.host).await;
        let page = crate::features::people::node_user_people_page::People {
            account: &account,
            people: &people,
            invitations: &invitations,
            projects: &projects,
            now,
            query_error: query.error.as_deref(),
            verify: verify.as_ref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::people::node_user_people_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }

    /// Accept an invitation. The one page a stranger may open.
    #[view("/join/:token")]
    #[middleware(SessionMiddleware)]
    async fn join(&self, query: JoinPage) -> UiResult<ViewOutcome> {
        // Somebody already signed in following an invitation link is
        // almost certainly the wrong person at the right machine.
        // Their session is not what the link is for.
        if signed_in(&self.auth).is_some() {
            return Ok(Redirect::found("/?error=sign+out+first+to+accept+an+invitation").into());
        }

        let invitation = invitations::look_up(&self.state.database, &query.token).await?;

        layout::head("Join");
        let passkey = match (
            invitation.is_some(),
            crate::infra::accounts::ceremonies::relying_party(&self.asked.host),
        ) {
            (true, Some(rp)) => Some(
                crate::infra::accounts::ceremonies::begin_registration(
                    &self.state.database,
                    &rp,
                    None,
                )
                .await?,
            ),
            _ => None,
        };
        let page = crate::features::people::guest_join_page::Join {
            passkey: passkey.as_ref(),
            heading: "Join this node",
            token: &query.token,
            invitation: invitation.as_ref(),
            error: query.error.as_deref(),
        };
        Ok(crate::features::people::guest_join_page::render(&page)
            .into_view()
            .into())
    }
}
