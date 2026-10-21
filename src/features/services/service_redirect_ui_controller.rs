use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::infra::repositories::{access, ports, services};
use crate::shell::auth::{signed_in, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Area, Frame};
use crate::shell::state::ConsoleState;

#[derive(Debug, Deserialize, Validate)]
pub struct NewRedirectQuery {
    pub project: String,
    pub service: String,
    pub error: Option<String>,
}

#[injectable]
pub struct ServiceRedirectUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl ServiceRedirectUiController {
    #[view("/projects/:project/services/:service/redirects/new")]
    #[middleware(SessionMiddleware)]
    async fn new_redirect(&self, query: NewRedirectQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, &query.service).await?
        else {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        };
        let back = format!(
            "/projects/{}/services/{}/settings/network",
            project.slug, service.slug
        );
        let targets: Vec<String> = ports::of_service(&self.state.database, &service.id)
            .await?
            .into_iter()
            .filter_map(|port| port.hostname)
            .collect();
        if !allowed.may_deploy() || !service.is_ours() || targets.is_empty() {
            return Ok(Redirect::found(back).into());
        }
        let all_projects = access::projects_for(&self.state.database, &account).await?;

        layout::head("Add redirect");
        let here = format!("/projects/{}/services/{}", project.slug, service.slug);
        let action = format!("{here}/redirects");
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("{here}/redirects/new"),
        )
        .allowing(allowed);
        let page = crate::features::services::new_service_redirect_page::NewRedirectPage {
            action: &action,
            back: &back,
            targets: &targets,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::services::new_service_redirect_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }
}
