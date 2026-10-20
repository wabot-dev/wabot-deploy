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
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ZoneQuery {
    pub zone: String,
    pub error: Option<String>,
}

#[injectable]
pub struct DnsUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl DnsUiController {
    #[view("/dns")]
    #[middleware(SessionMiddleware)]
    async fn zones(&self, query: ZonesQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }
        let table = self
            .state
            .container
            .try_resolve::<crate::infra::dns::Zones>()
            .ok();
        let zones = match crate::features::dns::node_user_see_zones::run(
            &self.state.database,
            table.as_deref(),
            &account,
        )
        .await
        {
            Ok(zones) => zones,
            Err(_) => return Ok(Redirect::found("/").into()),
        };
        let projects = access::projects_for(&self.state.database, &account).await?;

        layout::head("DNS");
        let frame = Frame::new(&account, Area::Dns, &projects, None, "/dns");
        let page = crate::features::dns::node_user_zones_page::ZonesPage {
            zones: &zones,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::dns::node_user_zones_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }

    #[view("/dns/:zone")]
    #[middleware(SessionMiddleware)]
    async fn zone(&self, query: ZoneQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }
        let zone = match crate::features::dns::node_user_see_zone::run(
            &self.state.database,
            &account,
            &query.zone,
        )
        .await
        {
            Ok(zone) => zone,
            Err(_) => return Ok(Redirect::found("/dns").into()),
        };
        let projects = access::projects_for(&self.state.database, &account).await?;

        layout::head(&zone.name);
        let path = format!("/dns/{}", zone.name);
        let frame = Frame::new(&account, Area::Dns, &projects, None, &path);
        let page = crate::features::dns::node_user_zone_page::ZonePage {
            zone: &zone,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::dns::node_user_zone_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }
}
