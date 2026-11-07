//! The OCI registry's routes: what `docker push` and `docker pull` talk
//! to.

use crate::infra::registry::*;

use crate::infra::repositories::projects;
use crate::infra::repositories::tokens;
use std::sync::Arc;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::axum::Router;
use wabot::rest::RestResult;

#[injectable]
pub struct RegistryRestController {
    state: Arc<RegistryState>,
}

#[rest_controller("/v2")]
impl RegistryRestController {
    /// The version check every client makes first.
    ///
    /// Answering `200` with the version header is what tells a client
    /// this is a v2 registry; answering `401` with a `WWW-Authenticate`
    /// is what makes it send credentials. Both matter: a `docker push`
    /// that never sees a challenge never sends the token.
    #[get("/")]
    #[raw]
    async fn version(&self, request: Request) -> RestResult<Response> {
        match self
            .authenticate(basic_password(&request), scope_of(&request))
            .await?
        {
            Some(_) => Ok(ok_empty()),
            None => Ok(challenge()),
        }
    }

    /// Is this blob already here?
    ///
    /// The question that makes a push fast on a node that shares one
    /// content store with the runtime: most layers of most images are
    /// already on disk because something else pulled them.
    #[head("/*path")]
    #[raw]
    async fn head(&self, request: Request) -> RestResult<Response> {
        let Some(pusher) = self
            .authenticate(basic_password(&request), scope_of(&request))
            .await?
        else {
            return Ok(challenge());
        };
        let path = request.uri().path().to_string();

        match Route::of(&path) {
            Some(Route::Blob { name, digest }) => {
                self.guard(&pusher, &name)?;
                blobs::head(&digest).await
            }
            Some(Route::Manifest { name, reference }) => {
                self.guard(&pusher, &name)?;
                manifests::head(&self.state, &name, &reference).await
            }
            _ => Ok(not_found()),
        }
    }

    #[get("/*path")]
    #[raw]
    async fn get(&self, request: Request) -> RestResult<Response> {
        let Some(pusher) = self
            .authenticate(basic_password(&request), scope_of(&request))
            .await?
        else {
            return Ok(challenge());
        };
        let path = request.uri().path().to_string();

        match Route::of(&path) {
            Some(Route::Blob { name, digest }) => {
                self.guard(&pusher, &name)?;
                blobs::get(&digest).await
            }
            Some(Route::Manifest { name, reference }) => {
                self.guard(&pusher, &name)?;
                manifests::get(&self.state, &name, &reference).await
            }
            _ => Ok(not_found()),
        }
    }

    /// Start an upload.
    #[post("/*path")]
    #[raw]
    async fn post(&self, request: Request) -> RestResult<Response> {
        let Some(pusher) = self
            .authenticate(basic_password(&request), scope_of(&request))
            .await?
        else {
            return Ok(challenge());
        };
        let path = request.uri().path().to_string();

        match Route::of(&path) {
            Some(Route::Uploads { name }) => {
                self.guard(&pusher, &name)?;
                blobs::start(&name, request).await
            }
            _ => Ok(not_found()),
        }
    }

    /// A chunk of an upload.
    #[patch("/*path")]
    #[raw]
    async fn patch(&self, request: Request) -> RestResult<Response> {
        let Some(pusher) = self
            .authenticate(basic_password(&request), scope_of(&request))
            .await?
        else {
            return Ok(challenge());
        };
        let path = request.uri().path().to_string();

        match Route::of(&path) {
            Some(Route::Upload { name, session }) => {
                self.guard(&pusher, &name)?;
                blobs::patch(&name, &session, request).await
            }
            _ => Ok(not_found()),
        }
    }

    /// Finish an upload, or put a manifest.
    #[put("/*path")]
    #[raw]
    async fn put(&self, request: Request) -> RestResult<Response> {
        let Some(pusher) = self
            .authenticate(basic_password(&request), scope_of(&request))
            .await?
        else {
            return Ok(challenge());
        };
        let path = request.uri().path().to_string();

        match Route::of(&path) {
            Some(Route::Upload { name, session }) => {
                self.guard(&pusher, &name)?;
                blobs::finish(&name, &session, request).await
            }
            Some(Route::Manifest { name, reference }) => {
                self.guard(&pusher, &name)?;
                manifests::put(&self.state, &pusher, &name, &reference, request).await
            }
            _ => Ok(not_found()),
        }
    }
}

impl RegistryRestController {
    /// Who is pushing, from the `Authorization` header.
    ///
    /// Basic auth with the push token as the password. The username is
    /// ignored — every registry client insists on sending one, and
    /// requiring a particular value would be one more thing to get
    /// wrong in a CI config for no security at all.
    /// Takes the credential rather than the request: holding a
    /// `&Request` across an `await` needs `Body: Sync`, which it is
    /// not, and the handler quietly stops being one axum accepts.
    ///
    /// On a project's own registry name only that project's tokens are
    /// anybody: the name exists so `docker login` keeps one credential
    /// per project, and another project's token answered there would put
    /// two projects behind one login again.
    async fn authenticate(
        &self,
        secret: Option<String>,
        scope: Option<String>,
    ) -> RestResult<Option<Pusher>> {
        let Some(secret) = secret else {
            return Ok(None);
        };
        let Some(project_id) = tokens::authenticate(&self.state.database, &secret).await? else {
            return Ok(None);
        };
        if scope.is_some_and(|scope| scope != project_id) {
            return Ok(None);
        }
        let Some(project) = projects::find(&self.state.database, &project_id).await? else {
            return Ok(None);
        };
        Ok(Some(Pusher { project }))
    }

    /// A token pushes to its own project and nowhere else.
    ///
    /// The repository name has to start with the project's slug, so
    /// `demo/api` belongs to `demo`. Without this a token would be a
    /// node-wide credential wearing a project's name.
    fn guard(&self, pusher: &Pusher, name: &str) -> RestResult<()> {
        let expected = &pusher.project.slug;
        let matches = name == expected.as_str()
            || name
                .strip_prefix(expected.as_str())
                .is_some_and(|rest| rest.starts_with('/'));

        if matches {
            Ok(())
        } else {
            Err(wabot::rest::RestError::Client {
                status: 403,
                message: format!("this token pushes to {expected}/… — {name:?} is somewhere else"),
            })
        }
    }
}

fn scope_of(request: &Request) -> Option<String> {
    request
        .extensions()
        .get::<crate::infra::edge::RegistryScope>()
        .map(|scope| scope.0.clone())
}

pub fn register(container: &Container, state: Arc<crate::infra::registry::RegistryState>) {
    container.register_instance::<crate::infra::registry::RegistryState>(state);
    register_transients!(container, RegistryRestController);
}

pub fn routes(container: &Container) -> Router {
    RegistryRestController::register_routes(container, Router::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;
    use wabot::rest::axum::body::Body;

    async fn version(router: &Router, secret: &str, scope: Option<&str>) -> u16 {
        use base64::Engine;
        let mut request = Request::builder()
            .uri("/v2")
            .header(
                "authorization",
                format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(format!("ci:{secret}"))
                ),
            )
            .body(Body::empty())
            .expect("request");
        if let Some(scope) = scope {
            request
                .extensions_mut()
                .insert(crate::infra::edge::RegistryScope(scope.to_string()));
        }
        router
            .clone()
            .oneshot(request)
            .await
            .expect("answered")
            .status()
            .as_u16()
    }

    /// On a project's own registry name, `docker login` with another
    /// project's token fails. That is what keeps one credential per
    /// project in the client: a host that took any project's token would
    /// be one login for two projects again. On the node's own name every
    /// project's token is still somebody.
    #[tokio::test]
    async fn a_projects_registry_name_takes_that_projects_tokens_only() {
        let database = Arc::new(crate::infra::db::open_in_memory().await.expect("open"));
        let acme = projects::create(&database, "acme").await.expect("project");
        let other = projects::create(&database, "other").await.expect("project");
        let (_, acme_secret) = tokens::create(&database, &acme.id, "ci", None)
            .await
            .expect("token");
        let (_, other_secret) = tokens::create(&database, &other.id, "ci", None)
            .await
            .expect("token");

        let container = Container::new();
        register(
            &container,
            Arc::new(crate::infra::registry::RegistryState {
                database: database.clone(),
                deployer: Arc::new(crate::infra::deploy::Deployer::new(
                    database.clone(),
                    &crate::infra::config::Config::default(),
                )),
            }),
        );
        let router = routes(&container);

        assert_eq!(version(&router, &acme_secret, Some(&acme.id)).await, 200);
        assert_eq!(version(&router, &other_secret, Some(&acme.id)).await, 401);
        assert_eq!(version(&router, &other_secret, None).await, 200);
        assert_eq!(version(&router, &acme_secret, None).await, 200);
    }
}
