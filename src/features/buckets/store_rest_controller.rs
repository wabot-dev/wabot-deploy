//! What the form that makes a store posts to.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::features::buckets::create_store_request::CreateStoreRequest;
use crate::shell::auth::{
    back_with_error, field, read_form, see_other, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct StoreRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl StoreRestController {
    #[post("/projects/:project/stores")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(project_slug) = crate::shell::auth::project_slug(&path) else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{project_slug}/stores/new");
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = CreateStoreRequest {
            project: project_slug.to_string(),
            name: field(&form, "name").to_string(),
            hostname: field(&form, "hostname").to_string(),
            memory: field(&form, "memory").to_string(),
            disk: field(&form, "disk").to_string(),
            allowlist: match crate::domain::values::allowlist::Allowlist::parse(field(
                &form,
                "allowlist",
            )) {
                Ok(allowlist) => allowlist,
                Err(message) => return Ok(back_with_error(&here, &message)),
            },
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&here, &reason.to_string()));
        }
        let made = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::buckets::node_user_create_store::run(
                    &self.state.database,
                    &account,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::buckets::tenant_user_create_store::run(
                    &self.state.database,
                    &account,
                    &asked,
                )
                .await
            }
        };
        let (project, service) = match made {
            Ok(made) => made,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        let command = crate::infra::deploy::jobs::DeployService::new(&service.id, None);
        if let Err(error) = wabot::async_jobs::run_command(&self.state.container, &command).await {
            tracing::error!(%error, "could not queue the deployment");
        }
        self.state.deployer.sync_routes().await;

        Ok(see_other(&format!(
            "/projects/{}/services/{}/buckets",
            project.slug, service.slug
        )))
    }
}

#[cfg(test)]
mod tests {
    use crate::web::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    #[tokio::test]
    async fn a_store_asked_for_while_the_node_has_them_off_is_sent_back_with_why() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        crate::infra::repositories::projects::create(&console.database, "shop")
            .await
            .expect("project");
        let response = console
            .harness
            .post("/projects/shop/stores")
            .header("cookie", &cookie)
            .form(&[("name", "files"), ("disk", "10 GB")])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/projects/shop/stores/new?error="),
            "{location}"
        );
        assert!(
            crate::infra::repositories::services::all(&console.database, None)
                .await
                .expect("services")
                .is_empty()
        );
    }
}
