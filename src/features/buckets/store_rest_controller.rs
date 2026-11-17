//! What the form that makes a store posts to.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::features::buckets::create_store_request::CreateStoreRequest;
use crate::features::buckets::restore_store_request::RestoreStoreRequest;
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

    /// Not under `…/buckets/`, where a POST-only `restore` would leave a
    /// bucket of that name with no page: axum answers 405 for the path
    /// before the bucket's view is reached.
    #[post("/projects/:project/stores/:store/restore")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn restore(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some((project, store)) = store_of(&path) else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{project}/services/{store}/buckets");
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = RestoreStoreRequest {
            project: project.clone(),
            store: store.clone(),
            at: field(&form, "at").to_string(),
            into: field(&form, "into").to_string(),
            hostname: field(&form, "hostname").to_string(),
        };
        let restored = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::buckets::node_user_restore_store::run(
                    &self.state.database,
                    &self.state.config,
                    &account,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::buckets::tenant_user_restore_store::run(
                    &self.state.database,
                    &self.state.config,
                    &account,
                    &asked,
                )
                .await
            }
        };
        let restored = match restored {
            Ok(restored) => restored,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        let command = crate::infra::deploy::jobs::DeployService::new(&restored.service.id, None);
        if let Err(error) = wabot::async_jobs::run_command(&self.state.container, &command).await {
            tracing::error!(service = %restored.service.id, %error, "could not queue a deployment");
        }
        self.state.deployer.sync_routes().await;
        tracing::info!(
            from = %store,
            into = %restored.service.slug,
            backup = %restored.from,
            taken_at = restored.taken_at,
            "restoring a store into a copy"
        );
        Ok(see_other(&format!(
            "/projects/{project}/services/{}/buckets",
            restored.service.slug
        )))
    }
}

fn store_of(path: &str) -> Option<(String, String)> {
    match crate::shell::auth::segments(path).as_slice() {
        ["projects", project, "stores", store, "restore"]
            if !project.is_empty() && !store.is_empty() =>
        {
            Some((project.to_string(), store.to_string()))
        }
        _ => None,
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

    async fn buckets_tab(console: &Console, cookie: &str) -> String {
        console
            .harness
            .get("/projects/shop/services/files/buckets")
            .header("cookie", cookie)
            .send()
            .await
            .body
    }

    /// The form is there only when a backup holds this store, and what it
    /// posts finds that backup before it makes anything: the refusal that
    /// comes back is about the copy's name, which is asked after.
    #[tokio::test]
    async fn a_store_is_offered_a_restore_only_once_a_backup_holds_it() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let store = crate::features::fixtures::store_in(&console.database, 1 << 30).await;

        let bare = buckets_tab(&console, &cookie).await;
        assert!(
            !bare.contains("/projects/shop/stores/files/restore"),
            "nothing to restore from, so nothing to press: {bare}"
        );

        let taken_at = crate::infra::clock::now_ms() - 3_600_000;
        let backup = console
            .data_dir
            .join("backups")
            .join("nodes")
            .join("nd-test")
            .join(taken_at.to_string());
        std::fs::create_dir_all(
            backup
                .join("volumes")
                .join("shop.files")
                .join(crate::domain::policies::garage::DATA_VOLUME)
                .join(crate::domain::policies::garage::METADATA_DIR),
        )
        .expect("a store's copy");
        std::fs::write(
            backup.join("manifest.json"),
            serde_json::json!({
                "format": 1,
                "taken_by": "wabot-deploy 0.0.0-test",
                "taken_at": taken_at,
                "node_id": "nd-test",
                "node_name": "test",
                "volumes": [{
                    "container": "shop.files",
                    "service": store.id,
                    "bytes": 20,
                    "how": "Consistent",
                }],
                "images": [],
            })
            .to_string(),
        )
        .expect("the manifest");

        let offered = buckets_tab(&console, &cookie).await;
        assert!(
            offered.contains("/projects/shop/stores/files/restore"),
            "now there is something to restore from: {offered}"
        );

        crate::infra::node::settings::set_buckets_enabled(&console.database, true)
            .await
            .expect("on");
        let response = console
            .harness
            .post("/projects/shop/stores/files/restore")
            .header("cookie", &cookie)
            .form(&[("at", ""), ("into", "files-friday"), ("hostname", "")])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/projects/shop/services/files/buckets?error=")
                && location.contains("domain"),
            "{location}"
        );
        assert_eq!(
            crate::infra::repositories::services::all(&console.database, None)
                .await
                .expect("services")
                .len(),
            1,
            "no copy was left half made"
        );
    }
}
