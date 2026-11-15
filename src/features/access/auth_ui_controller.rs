//! The pages somebody without a session can reach.
//!
//! Two, and they are the whole of what a stranger may see: the first
//! run's setup and the sign-in. What they post to is
//! `auth_rest_controller`; the helpers both of them use — reading a
//! form, redirecting, asking who is signed in — stay in `auth`, because
//! every controller in this console reads them.

use crate::infra::accounts::{self};
use crate::shell::layouts::layout;
use crate::shell::state::ConsoleState;
use std::sync::Arc;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::shell::auth::*;

// ---------- pages --------------------------------------------------------

/// Setup and sign-in. The only two pages a stranger may see.
#[injectable]
pub struct AuthUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl AuthUiController {
    /// Where somebody lands: what they came to look at.
    ///
    /// Whoever runs the machine opens it to see whether it is all right,
    /// so that is their first page; everybody else has projects and
    /// nothing else to see. A redirect rather than one address showing
    /// two pages, so each has a URL of its own and this one can be handed
    /// to anybody.
    #[view("/")]
    #[middleware(SessionMiddleware)]
    async fn home(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found(
                crate::shell::auth::signed_out_destination(&self.state).await,
            )
            .into());
        };
        let home = match crate::domain::authz::node_user::manages_the_machine(&account) {
            true => "/status",
            false => "/projects",
        };
        Ok(Redirect::found(crate::shell::auth::with_error(home, query.error.as_deref())).into())
    }

    /// Create the first administrator.
    ///
    /// Guarded twice over: the page is gone once an account exists, and
    /// the POST re-checks. The page going away is courtesy; the POST
    /// check is the security boundary, because a page is only a page.
    #[view("/setup")]
    #[middleware(SessionMiddleware)]
    async fn setup(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        if accounts::any_account(&self.state.database).await? {
            return Ok(Redirect::found("/sign-in").into());
        }
        // Without a token there is nothing to type, and saying so beats
        // a form that can only be refused.
        let has_token = accounts::setup_token_valid(&self.state.database).await?;

        layout::head("Set up");
        let page = crate::features::access::guest_setup_page::Setup {
            has_token,
            query_error: query.error.as_deref(),
        };
        Ok(crate::features::access::guest_setup_page::render(&page)
            .into_view()
            .into())
    }

    #[view("/sign-in")]
    #[middleware(SessionMiddleware)]
    async fn sign_in(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        if signed_in(&self.auth).is_some() {
            return Ok(Redirect::found("/").into());
        }
        // A node with nobody on it cannot be signed into, and sending
        // somebody to a form that will always refuse them is a worse
        // answer than sending them to the one that works.
        if !accounts::any_account(&self.state.database).await? {
            return Ok(Redirect::found("/setup").into());
        }

        layout::head("Sign in");
        let page = crate::features::access::guest_sign_in_page::SignIn {
            query_error: query.error.as_deref(),
        };
        Ok(crate::features::access::guest_sign_in_page::render(&page)
            .into_view()
            .into())
    }
}
