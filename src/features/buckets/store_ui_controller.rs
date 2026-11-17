//! The page that makes a store.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::infra::repositories::access;
use crate::shell::auth::{signed_in, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Frame, Section};
use crate::shell::state::ConsoleState;

#[derive(Debug, Deserialize, Validate)]
pub struct NewStoreForm {
    pub project: String,
    pub error: Option<String>,
}

#[injectable]
pub struct StoreUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl StoreUiController {
    #[view("/projects/:project/stores/new")]
    #[middleware(SessionMiddleware)]
    async fn new_store(&self, query: NewStoreForm) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/projects?error=no+such+project").into());
        };
        if !allowed.may_deploy() {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        }

        let domain = crate::infra::node::settings::domain(&self.state.database).await;
        let derived = crate::infra::repositories::project_domain::domain_for(
            &self.state.database,
            &project,
            domain.as_deref(),
        )
        .await
        .map(|suffix| {
            crate::infra::deploy::dns::suggested_hostname("<name>", &project.slug, &suffix)
        });
        let enabled = crate::infra::node::settings::buckets_enabled(&self.state.database).await;

        let action = format!("/projects/{}/stores", project.slug);
        let back = format!("/projects/{}", project.slug);
        let frame = Frame::load(
            &self.state.database,
            &account,
            Section::Projects,
            format!("/projects/{}/stores/new", project.slug),
        )
        .await;
        layout::head("Create store");
        let page = crate::features::buckets::new_store_page::NewStore {
            action: &action,
            back: &back,
            query_error: query.error.as_deref(),
            derived: derived.as_deref(),
            enabled,
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::buckets::new_store_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }
}
