use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::features::stability::see_service_stability_request::SeeServiceStabilityRequest;
use crate::features::stability::service_stability_page::{self, StabilityPage};
use crate::features::stability::stability_chart_component::plots;
use crate::features::stability::texts;
use crate::infra::repositories::{access, services};
use crate::shell::auth::{signed_in, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Frame, Section};
use crate::shell::state::ConsoleState;

/// Every field a string: a query parameter is text, and the framework hands
/// a field exactly what was in the address.
#[derive(Debug, Deserialize, Validate)]
pub struct StabilityQuery {
    pub project: String,
    pub service: String,
    pub days: Option<String>,
    pub day: Option<String>,
    pub event: Option<String>,
}

#[injectable]
pub struct ServiceStabilityUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
    asked: Arc<crate::shell::auth::Asked>,
}

#[ui_controller("/", app)]
impl ServiceStabilityUiController {
    #[view("/projects/:project/services/:service/stability")]
    #[middleware(SessionMiddleware)]
    async fn stability(&self, query: StabilityQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/projects?error=no+such+project").into());
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, &query.service).await?
        else {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        };
        let request = SeeServiceStabilityRequest::new(
            &project.slug,
            &service.slug,
            query.days.as_deref(),
            query.day.as_deref(),
            query.event.as_deref(),
            self.asked.zone.as_deref(),
        );
        let seen = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::stability::node_user_see_service_stability::run(
                    &self.state.database,
                    &account,
                    &request,
                )
                .await
            }
            false => {
                crate::features::stability::tenant_user_see_service_stability::run(
                    &self.state.database,
                    &account,
                    &request,
                )
                .await
            }
        };
        let path = format!("/projects/{}/services/{}", project.slug, service.slug);
        let response = match seen {
            Ok(response) => response,
            Err(reason) => {
                let reason = crate::features::services::service_push_component::urlencode(&reason);
                return Ok(Redirect::found(format!("{path}?error={reason}")).into());
            }
        };

        layout::head(&service.name);
        let frame = Frame::load(
            &self.state.database,
            &account,
            Section::Projects,
            format!("{path}/stability"),
        )
        .await;
        let page = StabilityPage {
            project_slug: &project.slug,
            service: &service,
            may_deploy: allowed.may_deploy(),
            response: &response,
        };
        let (body, data) = crate::shell::texts::language::scoped(account.language, || {
            texts::zoned(&response.zone, || {
                let drawn = plots(&response);
                let body = service_stability_page::render(&page, &drawn)
                    .render()
                    .into_inner();
                (body, service_stability_page::island_data(&page, &drawn))
            })
        });
        let body = wabot::ui::hypertext::island(
            "stability",
            &data,
            hypertext::Raw::dangerously_create(&body),
        )
        .render()
        .into_inner();
        Ok(frame.render(body).into_view().into())
    }
}
