//! Who is on this node, and the one page a stranger may open.

use std::sync::Arc;

use hypertext::prelude::*;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::accounts::{self, invitations};
use crate::policies::access;

use super::auth::{signed_in, JoinPage, PageQuery, SessionMiddleware};
use super::ConsoleState;
use crate::views::layouts::layout;
use crate::views::layouts::shell::{Area, Frame};

#[injectable]
pub struct PeopleUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
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
        if !crate::authz::node_user::manages_the_machine(&account) {
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
        let now = crate::console::now_ms();

        let frame = Frame::new(&account, Area::Settings, &projects, None, "/people");
        layout::head("People");

        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::views::pages::node_user_people_page::People {
            account: &account,
            people: &people,
            invitations: &invitations,
            projects: &projects,
            now,
            query_error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_people_page::render(&page)
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
        let page = crate::views::pages::guest_join_page::Join {
            heading: "Join this node",
            token: &query.token,
            invitation: invitation.as_ref(),
            error: query.error.as_deref(),
        };
        Ok(crate::views::pages::guest_join_page::render(&page)
            .into_view()
            .into())
    }
}
