//! What a store's bucket and key forms post to.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::features::buckets::create_bucket_key_request::CreateBucketKeyRequest;
use crate::features::buckets::create_bucket_request::CreateBucketRequest;
use crate::features::buckets::set_bucket_public_request::SetBucketPublicRequest;
use crate::features::buckets::set_bucket_rules_request::SetBucketRulesRequest;
use crate::features::buckets::store_converged_response::StoreConvergedResponse;
use crate::infra::repositories::object_stores;
use crate::shell::auth::{
    back_with_error, field, read_form, see_other, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct BucketRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

type Form = std::collections::HashMap<String, String>;

/// The store and, when the path names one, the bucket: the seventh segment
/// of `/projects/<p>/services/<s>/buckets/<b>/…`.
fn bucket_of(path: &str) -> Option<String> {
    match crate::shell::auth::segments(path).as_slice() {
        ["projects", _, "services", _, "buckets", bucket, ..] if !bucket.is_empty() => {
            Some(bucket.to_string())
        }
        _ => None,
    }
}

fn key_of(path: &str) -> Option<String> {
    match crate::shell::auth::segments(path).as_slice() {
        ["projects", _, "services", _, "buckets", _, "keys", key, ..] if !key.is_empty() => {
            Some(key.to_string())
        }
        _ => None,
    }
}

/// Where a change that saved its rows lands, saying whether the store took
/// it: the rows are kept either way, and a page that said "saved" while
/// Garage refused would be the row speaking instead of the store.
///
/// The card's fragment only on the way back from a success: a refusal's
/// query has to come before any fragment, and its note is at the top.
fn landed(here: &str, card: &str, converged: StoreConvergedResponse) -> Response {
    match converged {
        StoreConvergedResponse::Taken
        | StoreConvergedResponse::NotRunning
        | StoreConvergedResponse::Sent(_) => see_other(&format!("{here}{card}")),
        StoreConvergedResponse::Refused(reason) => back_with_error(
            here,
            &format!("saved, and the store did not take it yet: {reason}"),
        ),
    }
}

impl BucketRestController {
    /// The routes change with the bucket, so the edges are told now rather
    /// than at the next deployment, which a store does not need: a public
    /// bucket's own name carries its public reads.
    async fn publish(&self, request: Request, public: bool) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let found = match self.located(request).await? {
            Ok(found) => found,
            Err(response) => return Ok(response),
        };
        let (_, service, Some(bucket), here, form) = found else {
            return Ok(see_other("/"));
        };
        let asked = SetBucketPublicRequest {
            public,
            hostname: match public {
                true => field(&form, "hostname").to_string(),
                false => String::new(),
            },
        };
        let set = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::buckets::node_user_set_bucket_public::run(
                    &self.state.database,
                    &account,
                    &service,
                    &bucket,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::buckets::tenant_user_set_bucket_public::run(
                    &self.state.database,
                    &account,
                    &service,
                    &bucket,
                    &asked,
                )
                .await
            }
        };
        match set {
            Ok(converged) => {
                self.state.deployer.tell_edges(&service).await;
                self.state.deployer.sync_routes().await;
                Ok(landed(&here, "#public", converged))
            }
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    async fn located(
        &self,
        request: Request,
    ) -> RestResult<Result<(Project, Service, Option<Bucket>, String, Form), Response>> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) =
            crate::features::services::service_rest_controller::locate_for(
                &self.state,
                &self.auth,
                &path,
            )
            .await?
        else {
            return Ok(Err(see_other("/")));
        };
        let list = format!(
            "/projects/{}/services/{}/buckets",
            project.slug, service.slug
        );
        let bucket = match bucket_of(&path) {
            Some(name) => {
                match object_stores::bucket(&self.state.database, &service.id, &name).await? {
                    Some(bucket) => Some(bucket),
                    None => return Ok(Err(see_other(&list))),
                }
            }
            None => None,
        };
        let here = match &bucket {
            Some(bucket) => format!("{list}/{}", bucket.name),
            None => list,
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(Err(response)),
        };
        Ok(Ok((project, service, bucket, here, form)))
    }
}

#[rest_controller("/")]
impl BucketRestController {
    #[post("/projects/:project/services/:service/buckets")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let (_, service, _, here, form) = match self.located(request).await? {
            Ok(found) => found,
            Err(response) => return Ok(response),
        };
        let asked = CreateBucketRequest {
            name: field(&form, "name").to_string(),
            max_size: field(&form, "max_size").to_string(),
        };
        let made = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::buckets::node_user_create_bucket::run(
                    &self.state.database,
                    &account,
                    &service,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::buckets::tenant_user_create_bucket::run(
                    &self.state.database,
                    &account,
                    &service,
                    &asked,
                )
                .await
            }
        };
        match made {
            Ok((bucket, converged)) => {
                self.state.deployer.tell_edges(&service).await;
                self.state.deployer.sync_routes().await;
                Ok(landed(&format!("{here}/{}", bucket.name), "", converged))
            }
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/projects/:project/services/:service/buckets/:bucket/rules")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn rules(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let found = match self.located(request).await? {
            Ok(found) => found,
            Err(response) => return Ok(response),
        };
        let (_, service, Some(bucket), here, form) = found else {
            return Ok(see_other("/"));
        };
        let asked = SetBucketRulesRequest {
            max_size: field(&form, "max_size").to_string(),
            cors_origins: field(&form, "cors_origins").to_string(),
            expire_days: field(&form, "expire_days").to_string(),
            abort_uploads_days: field(&form, "abort_uploads_days").to_string(),
        };
        let set = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::buckets::node_user_set_bucket_rules::run(
                    &self.state.database,
                    &account,
                    &service,
                    &bucket,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::buckets::tenant_user_set_bucket_rules::run(
                    &self.state.database,
                    &account,
                    &service,
                    &bucket,
                    &asked,
                )
                .await
            }
        };
        match set {
            Ok(converged) => Ok(landed(&here, "#rules", converged)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/projects/:project/services/:service/buckets/:bucket/public")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn public(&self, request: Request) -> RestResult<Response> {
        self.publish(request, true).await
    }

    #[post("/projects/:project/services/:service/buckets/:bucket/private")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn private(&self, request: Request) -> RestResult<Response> {
        self.publish(request, false).await
    }

    /// Typing the name is the confirmation, checked here.
    #[post("/projects/:project/services/:service/buckets/:bucket/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let found = match self.located(request).await? {
            Ok(found) => found,
            Err(response) => return Ok(response),
        };
        let (project, service, Some(bucket), here, form) = found else {
            return Ok(see_other("/"));
        };
        if field(&form, "confirm").trim() != bucket.name.as_str() {
            return Ok(back_with_error(
                &here,
                "type the bucket's name to confirm deleting it",
            ));
        }
        let deleted = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::buckets::node_user_delete_bucket::run(
                    &self.state.database,
                    &account,
                    &service,
                    &bucket,
                )
                .await
            }
            false => {
                crate::features::buckets::tenant_user_delete_bucket::run(
                    &self.state.database,
                    &account,
                    &service,
                    &bucket,
                )
                .await
            }
        };
        match deleted {
            Ok(()) => {
                self.state.deployer.sync_routes().await;
                Ok(see_other(&format!(
                    "/projects/{}/services/{}/buckets",
                    project.slug, service.slug
                )))
            }
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    /// The secret travels to the page by a nonce and is shown once — see
    /// `ConsoleState::reveals`.
    #[post("/projects/:project/services/:service/buckets/:bucket/keys")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create_key(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let browser = crate::shell::auth::browser(&request);
        let found = match self.located(request).await? {
            Ok(found) => found,
            Err(response) => return Ok(response),
        };
        let (_, service, Some(bucket), here, form) = found else {
            return Ok(see_other("/"));
        };
        if let Err(reason) =
            crate::shell::auth::proven(&self.state, &self.auth, &browser, &form).await
        {
            return Ok(back_with_error(&here, &reason));
        }
        let asked = CreateBucketKeyRequest {
            name: field(&form, "name").to_string(),
            access: field(&form, "access").to_string(),
        };
        let made = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::buckets::node_user_create_bucket_key::run(
                    &self.state.database,
                    &account,
                    &service,
                    &bucket,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::buckets::tenant_user_create_bucket_key::run(
                    &self.state.database,
                    &account,
                    &service,
                    &bucket,
                    &asked,
                )
                .await
            }
        };
        let key = match made {
            Ok(key) => key,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        let endpoint =
            crate::infra::repositories::ports::of_service(&self.state.database, &service.id)
                .await?
                .into_iter()
                .find(|port| port.container_port == crate::domain::policies::garage::S3_PORT)
                .and_then(|port| port.hostname)
                .map(|hostname| {
                    format!(
                        "https://{}",
                        crate::infra::node::authority(
                            Some(&hostname),
                            self.state.config.edge.https_port
                        )
                    )
                })
                .unwrap_or_default();
        let nonce = self.state.reveals.stash(format!(
            "AWS_ACCESS_KEY_ID={}\nAWS_SECRET_ACCESS_KEY={}\nAWS_ENDPOINT_URL={endpoint}\nAWS_REGION={}\nBUCKET={}",
            key.access_key_id,
            key.secret_key,
            crate::domain::policies::garage::REGION,
            bucket.name,
        ));
        Ok(see_other(&format!(
            "{here}/shown?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("nonce", &nonce)
                .finish()
        )))
    }

    #[post("/projects/:project/services/:service/buckets/:bucket/keys/:key/revoke")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revoke_key(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let found = match self.located(request).await? {
            Ok(found) => found,
            Err(response) => return Ok(response),
        };
        let (_, service, Some(bucket), here, _) = found else {
            return Ok(see_other("/"));
        };
        let Some(key) = object_stores::keys(&self.state.database, &bucket.id)
            .await?
            .into_iter()
            .find(|key| Some(&key.id) == key_of(&path).as_ref())
        else {
            return Ok(see_other(&format!("{here}#keys")));
        };
        let revoked = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::buckets::node_user_delete_bucket_key::run(
                    &self.state.database,
                    &account,
                    &service,
                    &key,
                )
                .await
            }
            false => {
                crate::features::buckets::tenant_user_delete_bucket_key::run(
                    &self.state.database,
                    &account,
                    &service,
                    &key,
                )
                .await
            }
        };
        match revoked {
            Ok(()) => Ok(see_other(&format!("{here}#keys"))),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bucket_and_the_key_are_read_from_where_the_path_says() {
        let path = "/projects/shop/services/files/buckets/uploads/keys/bk-1/revoke";
        assert_eq!(bucket_of(path).as_deref(), Some("uploads"));
        assert_eq!(key_of(path).as_deref(), Some("bk-1"));
        assert_eq!(bucket_of("/projects/shop/services/files/buckets"), None);
        assert_eq!(
            key_of("/projects/shop/services/files/buckets/uploads/rules"),
            None
        );
    }
}
