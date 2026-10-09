//! A secret, shown once, on a page of its own.
//!
//! ## Why four routes and one page
//!
//! What the page says depends on which secret it is — a join token is a
//! command, an invitation is a link, a push token has a username beside
//! it — and the route is what knows. So the kind comes from the path
//! and the secret comes from the nonce, which is the split that keeps
//! the store holding nothing but bytes.
//!
//! And the path is what the breadcrumb reads, which is the other half:
//! `/organisations/tnt-x/shown` puts the organisation in the trail and
//! the arrow on it, so the way back is the console's own way back
//! rather than a button this page draws — see
//! `no_page_offers_a_way_back_the_breadcrumb_already_is`.
//!
//! ## The nonce, not the secret
//!
//! The redirect carries a nonce that names the secret; the secret never
//! reaches the address bar, the history, or a refresh. Three invitation
//! links did travel in the query string until this existed, which is
//! the exception `ConsoleState::reveals` was written for and did not
//! cover.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use super::auth::{signed_in, SessionMiddleware};
use super::ConsoleState;
use crate::web::views::layouts::layout;
use crate::web::views::layouts::shell::{Area, Frame};
use crate::web::views::pages::revealed_page::{Revealed, What};

#[derive(Debug, Deserialize, Validate, Default)]
pub struct ShownPage {
    /// Names the secret. Not the secret.
    pub nonce: Option<String>,
}

#[derive(Debug, Deserialize, Validate, Default)]
pub struct ShownForPage {
    pub id: String,
    pub nonce: Option<String>,
}

#[injectable]
pub struct RevealUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

impl RevealUiController {
    /// Render one, or send them back where they came from.
    ///
    /// **A spent nonce is not an error.** It is what a refresh looks
    /// like, and what a browser that restored a tab looks like. The
    /// answer is the page the arrow points at, which is where they were
    /// going.
    /// `here` is this page's own path, and `back` is where both the
    /// arrow and a spent nonce go — **the same place, on purpose.** A
    /// refresh landing somewhere the arrow does not point is two answers
    /// to "where was I", and the arrow's destination is the breadcrumb's
    /// to compute, so `back` is written to agree with it rather than
    /// chosen freely.
    fn show(
        &self,
        account: &crate::domain::entities::account::Account,
        nonce: Option<&str>,
        here: &str,
        back: &str,
        area: Area,
        what: impl FnOnce() -> WhatKind,
    ) -> ViewOutcome {
        let Some(secret) = nonce.and_then(|nonce| self.state.reveals.take(nonce)) else {
            return Redirect::found(back.to_string()).into();
        };
        let kind = what();

        layout::head("Shown once");
        let frame = Frame::new(account, area, &[], None, here.to_string());
        let page = Revealed {
            what: kind.borrow(),
            secret: &secret,
        };
        let body = crate::web::views::texts::language::scoped(account.language, || {
            crate::web::views::pages::revealed_page::render(&page)
                .render()
                .into_inner()
        });
        frame.render(body).into_view().into()
    }
}

/// The owned half of [`What`], because the borrowed one cannot outlive
/// the `match` that built it.
pub enum WhatKind {
    JoinToken,
    InvitationInto(String),
    Invitation,
    PushToken,
}

impl WhatKind {
    fn borrow(&self) -> What<'_> {
        match self {
            WhatKind::JoinToken => What::JoinToken,
            WhatKind::InvitationInto(name) => What::InvitationInto(name),
            WhatKind::Invitation => What::Invitation,
            WhatKind::PushToken => What::PushToken,
        }
    }
}

#[ui_controller("/", app)]
impl RevealUiController {
    /// A token for a node that is not here yet.
    #[view("/network/invite/shown")]
    #[middleware(SessionMiddleware)]
    async fn join_token(&self, query: ShownPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }
        Ok(self.show(
            &account,
            query.nonce.as_deref(),
            "/network/invite/shown",
            "/network/invite",
            Area::Network,
            || WhatKind::JoinToken,
        ))
    }

    /// A link into an organisation, from the node's own console.
    #[view("/organisations/:id/shown")]
    #[middleware(SessionMiddleware)]
    async fn organisation_invitation(&self, query: ShownForPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::domain::authz::node_user::shares_the_machine_out(&account) {
            return Ok(Redirect::found("/").into());
        }
        let back = format!("/organisations/{}", query.id);
        let here = format!("{back}/shown");
        let Some(tenant) =
            crate::infra::repositories::tenants::find(&self.state.database, &query.id).await?
        else {
            return Ok(Redirect::found("/organisations").into());
        };
        Ok(self.show(
            &account,
            query.nonce.as_deref(),
            &here,
            &back,
            Area::Settings,
            || WhatKind::InvitationInto(tenant.name),
        ))
    }

    /// A link into whichever console this is.
    #[view("/people/shown")]
    #[middleware(SessionMiddleware)]
    async fn person_invitation(&self, query: ShownPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        Ok(self.show(
            &account,
            query.nonce.as_deref(),
            "/people/shown",
            "/people",
            Area::Settings,
            || WhatKind::Invitation,
        ))
    }

    /// A registry credential for one project.
    #[view("/projects/:id/shown")]
    #[middleware(SessionMiddleware)]
    async fn push_token(&self, query: ShownForPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        // The project, not its settings: `settings` is a verb, so the
        // breadcrumb ends there and the arrow it draws points at the
        // project. This is the value that agrees with it.
        let back = format!("/projects/{}", query.id);
        let here = format!("{back}/shown");
        Ok(self.show(
            &account,
            query.nonce.as_deref(),
            &here,
            &back,
            Area::Projects,
            || WhatKind::PushToken,
        ))
    }
}
