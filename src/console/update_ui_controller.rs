//! What this node is running, and what it could be.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::policies::access;
use crate::update::runs::{self};

use super::auth::{signed_in, SessionMiddleware};
use super::ConsoleState;
use crate::views::layouts::layout;
use crate::views::layouts::shell::{Area, Frame};

#[derive(Debug, Deserialize, Validate)]
pub struct UpdatesPage {
    pub error: Option<String>,
    pub started: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ReleasePage {
    pub tag: String,
    pub error: Option<String>,
}

#[injectable]
pub struct UpdateUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl UpdateUiController {
    #[view("/updates")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: UpdatesPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        // Updating the node is the node's business, which belongs to
        // whoever runs it.
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        let projects = access::projects_for(&self.state.database, &account).await?;
        let history = runs::recent(&self.state.database, 5)
            .await
            .unwrap_or_else(|error| {
                // The release list and the version are the page; a
                // history that cannot be read is a missing card, not a
                // missing page.
                tracing::warn!(%error, "could not read the update history");
                Vec::new()
            });
        let latest = history.first().cloned();

        // A failure to reach GitHub is not a failure of the page: this
        // node runs fine without ever asking, and the page still has
        // to say what version it is and what happened last time.
        let available = crate::update::availability(&self.state.catalogue).await;

        layout::head("Updates");
        let frame = Frame::new(&account, Area::Settings, &projects, None, "/updates");
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::views::pages::node_user_updates_page::Updates {
            history: &history,
            latest: latest.as_ref(),
            available: &available,
            query_error: query.error.as_deref(),
            query_started: query.started.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_updates_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// One release, in full — the notes are the reason this page
    /// exists rather than a version number on a button.
    #[view("/updates/:tag")]
    #[middleware(SessionMiddleware)]
    async fn release(&self, query: ReleasePage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        let releases = match self.state.catalogue.releases().await {
            Ok(releases) => releases,
            Err(error) => {
                let back = format!(
                    "/updates?{}",
                    super::auth::encode("error", &error.to_string())
                );
                return Ok(Redirect::found(back).into());
            }
        };
        let Some(release) = crate::update::github::find(&releases, &query.tag).cloned() else {
            return Ok(Redirect::found("/updates").into());
        };

        let projects = access::projects_for(&self.state.database, &account).await?;
        let current = crate::update::github::Version::current();

        layout::head(&format!("wabot-deploy {}", release.version));
        let frame = Frame::new(&account, Area::Settings, &projects, None, "/updates");
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::views::pages::node_user_release_page::ReleaseView {
            release: &release,
            current,
            query_error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_release_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}
