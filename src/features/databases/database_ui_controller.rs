//! A managed database's own pages.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::infra::repositories::access;
use crate::shell::auth::{signed_in, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Area, Frame};
use crate::shell::state::ConsoleState;

#[derive(Debug, Deserialize, Validate)]
pub struct DatabaseForm {
    pub project: String,
    pub error: Option<String>,
}

#[injectable]
pub struct DatabaseUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl DatabaseUiController {
    #[view("/projects/:project/databases/new")]
    #[middleware(SessionMiddleware)]
    async fn new_database(&self, query: DatabaseForm) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        if !allowed.may_deploy() {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        }

        let action = format!("/projects/{}/databases", project.slug);
        let back = format!("/projects/{}", project.slug);
        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("/projects/{}/databases/new", project.slug),
        )
        .allowing(allowed);

        layout::head("Create database");
        let page = crate::features::databases::new_database_page::NewDatabase {
            action: &action,
            back: &back,
            query_error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::databases::new_database_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}
