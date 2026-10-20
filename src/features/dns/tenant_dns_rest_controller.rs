use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::shell::auth::{
    back_with_error, read_form, see_other, segments, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct TenantDnsRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl TenantDnsRestController {
    #[post("/dns/zones")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create_zone(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = crate::features::dns::create_zone_request::CreateZoneRequest {
            name: form.get("name").cloned().unwrap_or_default(),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error("/dns", &reason.to_string()));
        }
        match crate::features::dns::tenant_user_add_domain::run(
            &self.state.database,
            &self.state.container,
            &account,
            &asked,
        )
        .await
        {
            Ok(created) => Ok(see_other(&format!("/dns/{}", created.name))),
            Err(reason) => Ok(back_with_error("/dns", &reason)),
        }
    }

    #[post("/dns/:zone/records")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add_record(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(zone) = segments(&path).get(1).map(|zone| zone.to_string()) else {
            return Ok(see_other("/dns"));
        };
        let here = format!("/dns/{zone}");
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let field = |name: &str| form.get(name).cloned().unwrap_or_default();
        let asked = crate::features::dns::add_record_request::AddRecordRequest {
            zone,
            name: field("name"),
            kind: field("kind"),
            value: field("value"),
            priority: field("priority"),
            ttl: field("ttl"),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&here, &reason.to_string()));
        }
        match crate::features::dns::tenant_user_add_record::run(
            &self.state.database,
            &self.state.container,
            &account,
            &asked,
        )
        .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/dns/:zone/records/:record/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn remove_record(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let parts = segments(&path);
        let (Some(zone), Some(record)) = (parts.get(1), parts.get(3)) else {
            return Ok(see_other("/dns"));
        };
        let here = format!("/dns/{zone}");
        match crate::features::dns::tenant_user_remove_record::run(
            &self.state.database,
            &self.state.container,
            &account,
            zone,
            record,
        )
        .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/dns/:zone/check")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn check_delegation(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(zone) = segments(&path).get(1).map(|zone| zone.to_string()) else {
            return Ok(see_other("/dns"));
        };
        let here = format!("/dns/{zone}");
        match crate::features::dns::tenant_user_check_delegation::run(
            &self.state.database,
            &account,
            &zone,
        )
        .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/dns/:zone/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete_zone(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(zone) = segments(&path).get(1).map(|zone| zone.to_string()) else {
            return Ok(see_other("/dns"));
        };
        match crate::features::dns::tenant_user_delete_zone::run(
            &self.state.database,
            &self.state.container,
            &account,
            &zone,
        )
        .await
        {
            Ok(()) => Ok(see_other("/dns")),
            Err(reason) => Ok(back_with_error(&format!("/dns/{zone}"), &reason)),
        }
    }
}
