//! An OCI registry that shares containerd's storage.
//!
//! ## Why the node hosts one at all
//!
//! Without it, deploying means pushing to somebody else's registry and
//! pulling it back — the bytes cross the internet twice and the node
//! depends on a service it does not run. With it, `docker push` puts
//! the image where the runtime will read it, and there is no pull.
//!
//! ## Sharing the content store, not copying into it
//!
//! Blobs go straight into containerd's content store through its
//! Content service, and the manifest becomes a containerd image
//! record. So a pushed image is *already* the image the runtime runs:
//! no second copy on disk, no import step, and `ctr images ls` shows
//! what the console shows.
//!
//! That is the whole reason this is worth building rather than running
//! a registry container beside containerd — which would store every
//! layer twice on a node whose whole design is about not doing that.
//!
//! ## What is implemented
//!
//! The push half of the distribution spec, which is what a node
//! receiving deployments needs: version check, blob existence, chunked
//! and monolithic uploads, manifests, and the pull half needed to make
//! `docker pull` from this node work. Not catalogs, not cross-repo
//! mounts, not deletes.

pub(crate) mod blobs;
pub(crate) mod manifests;

use std::sync::Arc;

use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, HeaderValue, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::sqlite::SqliteDatabase;

/// What the registry needs to answer a request.
pub struct RegistryState {
    pub database: Arc<SqliteDatabase>,
    pub deployer: Arc<crate::infra::deploy::Deployer>,
}

impl RegistryState {
    /// The host an image pushed here is named under, so `ctr images ls`
    /// shows something somebody could paste into a `docker pull`.
    ///
    /// Asked at each push. It was read once at startup, so a node named
    /// from the console after it booted named every push `localhost/…` —
    /// and a backup, which decides what is this node's own by its current
    /// name, then kept none of them. Found on a VM node.
    pub async fn host(&self) -> String {
        crate::infra::node::settings::domain(&self.database)
            .await
            .unwrap_or_else(|| "localhost".into())
    }
}

/// A caller that proved it may push to a project.
pub struct Pusher {
    pub project: crate::domain::entities::project::Project,
}

/// What a `/v2/…` path means.
///
/// Parsed by hand because the repository name may contain slashes,
/// which is exactly what a router's path parameters cannot express:
/// `demo/api/blobs/sha256:…` has to split at the *last* `/blobs/`, not
/// the first `/`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// `/v2/<name>/blobs/uploads/`
    Uploads { name: String },
    /// `/v2/<name>/blobs/uploads/<session>`
    Upload { name: String, session: String },
    /// `/v2/<name>/blobs/<digest>`
    Blob { name: String, digest: String },
    /// `/v2/<name>/manifests/<reference>`
    Manifest { name: String, reference: String },
}

impl Route {
    pub(crate) fn of(path: &str) -> Option<Self> {
        let path = path.strip_prefix("/v2/")?;

        if let Some((name, rest)) = split_last(path, "/blobs/uploads/") {
            return Some(if rest.is_empty() {
                Route::Uploads { name }
            } else {
                Route::Upload {
                    name,
                    session: rest,
                }
            });
        }
        if let Some((name, rest)) = split_last(path, "/blobs/uploads") {
            if rest.is_empty() {
                return Some(Route::Uploads { name });
            }
        }
        if let Some((name, digest)) = split_last(path, "/blobs/") {
            return Some(Route::Blob { name, digest });
        }
        if let Some((name, reference)) = split_last(path, "/manifests/") {
            return Some(Route::Manifest { name, reference });
        }
        None
    }
}

/// Split on the last occurrence of `separator`.
///
/// The last, because a repository name may itself contain anything
/// that looks like the separator's first segment — and because the
/// part after it never does.
fn split_last(path: &str, separator: &str) -> Option<(String, String)> {
    let at = path.rfind(separator)?;
    let name = &path[..at];
    if name.is_empty() {
        return None;
    }
    Some((name.to_string(), path[at + separator.len()..].to_string()))
}

/// The password out of a `Basic` `Authorization` header.
pub(crate) fn basic_password(request: &Request) -> Option<String> {
    use base64::Engine;

    let header = request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let encoded = header.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .ok()?;
    let pair = String::from_utf8(decoded).ok()?;
    // Split on the first colon: a password may contain one, a username
    // may not.
    Some(pair.split_once(':')?.1.to_string())
}

/// `401` with the challenge that makes a client send credentials.
///
/// Without the header a `docker push` gives up rather than retrying
/// with the token it already has — the client waits to be asked.
pub(crate) fn challenge() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"wabot-deploy\""),
        )
        .header("docker-distribution-api-version", "registry/2.0")
        .body(Body::empty())
        .expect("a constant response is well-formed")
}

pub(crate) fn ok_empty() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("docker-distribution-api-version", "registry/2.0")
        .body(Body::empty())
        .expect("a constant response is well-formed")
}

pub(crate) fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("docker-distribution-api-version", "registry/2.0")
        .body(Body::empty())
        .expect("a constant response is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node named from its console after it booted names the next push
    /// after that name. It named every one `localhost/…` for as long as
    /// the process lived, and a backup then kept none of them.
    #[tokio::test]
    async fn a_push_is_named_after_the_name_the_node_has_now() {
        let database = Arc::new(crate::infra::db::open_in_memory().await.expect("open"));
        let state = RegistryState {
            database: database.clone(),
            deployer: Arc::new(crate::infra::deploy::Deployer::new(
                database.clone(),
                &crate::infra::config::Config::default(),
            )),
        };
        assert_eq!(state.host().await, "localhost");

        crate::infra::node::settings::set_domain(&database, Some("node.example"))
            .await
            .expect("named");
        assert_eq!(state.host().await, "node.example");
    }

    #[test]
    fn a_repository_name_may_contain_slashes() {
        assert_eq!(
            Route::of("/v2/demo/api/blobs/sha256:abc"),
            Some(Route::Blob {
                name: "demo/api".into(),
                digest: "sha256:abc".into()
            })
        );
        assert_eq!(
            Route::of("/v2/demo/team/api/manifests/v1"),
            Some(Route::Manifest {
                name: "demo/team/api".into(),
                reference: "v1".into()
            })
        );
    }

    #[test]
    fn an_upload_is_told_apart_from_a_blob() {
        assert_eq!(
            Route::of("/v2/demo/api/blobs/uploads/"),
            Some(Route::Uploads {
                name: "demo/api".into()
            })
        );
        assert_eq!(
            Route::of("/v2/demo/api/blobs/uploads/abc-123"),
            Some(Route::Upload {
                name: "demo/api".into(),
                session: "abc-123".into()
            })
        );
    }

    /// Some clients POST without the trailing slash.
    #[test]
    fn an_upload_start_is_recognised_either_way() {
        assert_eq!(
            Route::of("/v2/demo/api/blobs/uploads"),
            Some(Route::Uploads {
                name: "demo/api".into()
            })
        );
    }

    #[test]
    fn nothing_useful_comes_out_of_a_path_that_is_not_ours() {
        assert_eq!(Route::of("/v2/"), None);
        assert_eq!(Route::of("/healthz"), None);
        assert_eq!(Route::of("/v2/demo/api"), None);
        // A name is required: `/v2//blobs/…` names nothing.
        assert_eq!(Route::of("/v2//blobs/sha256:abc"), None);
    }

    #[test]
    fn the_password_comes_out_of_a_basic_header() {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode("anything:s3cret");
        let request = Request::builder()
            .uri("/v2/")
            .header(header::AUTHORIZATION, format!("Basic {encoded}"))
            .body(Body::empty())
            .unwrap();

        assert_eq!(basic_password(&request).as_deref(), Some("s3cret"));
    }

    /// A token can contain a colon, and splitting on every one would
    /// hand containerd half a secret.
    #[test]
    fn a_password_may_contain_a_colon() {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode("user:a:b:c");
        let request = Request::builder()
            .uri("/v2/")
            .header(header::AUTHORIZATION, format!("Basic {encoded}"))
            .body(Body::empty())
            .unwrap();

        assert_eq!(basic_password(&request).as_deref(), Some("a:b:c"));
    }

    #[test]
    fn no_header_is_no_password() {
        let request = Request::builder().uri("/v2/").body(Body::empty()).unwrap();
        assert_eq!(basic_password(&request), None);
    }

    /// Without the challenge header a client gives up instead of
    /// retrying with the credentials it already holds.
    #[test]
    fn the_refusal_asks_for_credentials() {
        let response = challenge();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().get(header::WWW_AUTHENTICATE).is_some());
    }
}
