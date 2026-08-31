//! The OCI registry's routes: what `docker push` and `docker pull` talk
//! to.

use super::*;

use crate::repositories::projects;
use crate::repositories::tokens;
use std::sync::Arc;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
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
        match self.authenticate(basic_password(&request)).await? {
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
        let Some(pusher) = self.authenticate(basic_password(&request)).await? else {
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
        let Some(pusher) = self.authenticate(basic_password(&request)).await? else {
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
        let Some(pusher) = self.authenticate(basic_password(&request)).await? else {
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
        let Some(pusher) = self.authenticate(basic_password(&request)).await? else {
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
        let Some(pusher) = self.authenticate(basic_password(&request)).await? else {
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
    async fn authenticate(&self, secret: Option<String>) -> RestResult<Option<Pusher>> {
        let Some(secret) = secret else {
            return Ok(None);
        };
        let Some(project_id) = tokens::authenticate(&self.state.database, &secret).await? else {
            return Ok(None);
        };
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
