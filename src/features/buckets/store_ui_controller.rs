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
        let (here, elsewhere): (Vec<_>, Vec<_>) = crate::infra::network::candidates_for(
            &self.state.database,
            crate::domain::values::capability::Capability::Store,
        )
        .await?
        .into_iter()
        .filter(|node| {
            node.is_self || crate::infra::network::errand::holds_stores(node.version.as_deref())
        })
        .partition(|node| node.is_self);
        let here = match here.into_iter().next() {
            Some(node) => Some(node.name),
            None => crate::infra::network::me(&self.state.database)
                .await?
                .map(|node| node.name),
        };

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
            here: here.as_deref(),
            elsewhere: &elsewhere,
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::buckets::new_store_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }
}
