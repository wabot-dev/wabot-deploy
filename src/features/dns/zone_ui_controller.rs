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
pub struct ZonesQuery {
    pub project: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ZoneQuery {
    pub project: String,
    pub zone: String,
    pub error: Option<String>,
}

#[injectable]
pub struct ZoneUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl ZoneUiController {
    #[view("/projects/:project/zones")]
    #[middleware(SessionMiddleware)]
    async fn zones(&self, query: ZonesQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let seen = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::dns::node_user_see_zones::run(
                    &self.state.database,
                    &account,
                    &project.slug,
                )
                .await
            }
            false => {
                crate::features::dns::tenant_user_see_zones::run(
                    &self.state.database,
                    &account,
                    &project.slug,
                )
                .await
            }
        };
        let Ok(zones) = seen else {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        };
        let all_projects = access::projects_for(&self.state.database, &account).await?;

        layout::head("DNS");
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("/projects/{}/zones", project.slug),
        )
        .allowing(allowed);
        let page = crate::features::dns::zones_page::ZonesPage {
            project: &project,
            zones: &zones,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::dns::zones_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }

    #[view("/projects/:project/zones/new")]
    #[middleware(SessionMiddleware)]
    async fn new_zone(&self, query: ZonesQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let back = format!("/projects/{}/zones", project.slug);
        let seen = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::dns::node_user_see_zones::run(
                    &self.state.database,
                    &account,
                    &project.slug,
                )
                .await
            }
            false => {
                crate::features::dns::tenant_user_see_zones::run(
                    &self.state.database,
                    &account,
                    &project.slug,
                )
                .await
            }
        };
        let Ok(zones) = seen else {
            return Ok(Redirect::found(back).into());
        };
        if !zones.editable {
            return Ok(Redirect::found(back).into());
        }
        let all_projects = access::projects_for(&self.state.database, &account).await?;

        layout::head("Add zone");
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("{back}/new"),
        )
        .allowing(allowed);
        let page = crate::features::dns::new_zone_page::NewZonePage {
            action: &back,
            back: &back,
            name_servers: &zones.name_servers,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::dns::new_zone_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }

    #[view("/projects/:project/zones/:zone")]
    #[middleware(SessionMiddleware)]
    async fn zone(&self, query: ZoneQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let seen = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::dns::node_user_see_zone::run(
                    &self.state.database,
                    &account,
                    &project.slug,
                    &query.zone,
                )
                .await
            }
            false => {
                crate::features::dns::tenant_user_see_zone::run(
                    &self.state.database,
                    &account,
                    &project.slug,
                    &query.zone,
                )
                .await
            }
        };
        let Ok(zone) = seen else {
            return Ok(Redirect::found(format!("/projects/{}/zones", project.slug)).into());
        };
        let all_projects = access::projects_for(&self.state.database, &account).await?;

        layout::head(&zone.name);
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("/projects/{}/zones/{}", project.slug, zone.name),
        )
        .allowing(allowed);
        let page = crate::features::dns::zone_page::ZonePage {
            project: &project,
            zone: &zone,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::dns::zone_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }

    #[view("/projects/:project/zones/:zone/records/new")]
    #[middleware(SessionMiddleware)]
    async fn new_record(&self, query: ZoneQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let found = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::dns::node_user_see_zone::theirs(
                    &self.state.database,
                    &account,
                    &query.project,
                    &query.zone,
                )
                .await
            }
            false => {
                crate::features::dns::tenant_user_see_zone::theirs(
                    &self.state.database,
                    &account,
                    &query.project,
                    &query.zone,
                )
                .await
            }
        };
        let Ok((project, allowed, zone)) = found else {
            return Ok(Redirect::found(format!("/projects/{}/zones", query.project)).into());
        };
        let back = format!("/projects/{}/zones/{}", project.slug, zone.name);
        if !allowed.may_deploy() || !zone.is_ours() {
            return Ok(Redirect::found(back).into());
        }
        let all_projects = access::projects_for(&self.state.database, &account).await?;

        layout::head("Add record");
        let action = format!("{back}/records");
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("{back}/records/new"),
        )
        .allowing(allowed);
        let zone_name = zone.name.to_string();
        let page = crate::features::dns::new_record_page::NewRecordPage {
            zone: &zone_name,
            action: &action,
            back: &back,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::dns::new_record_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }
}
