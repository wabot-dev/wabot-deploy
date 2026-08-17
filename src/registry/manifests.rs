//! Manifests: the end of a push, and the start of a release.
//!
//! A manifest is itself a blob — content-addressed like any other —
//! and it is also what makes the pile of layers an image. So putting
//! one does three things: stores the bytes, creates the containerd
//! image record that names them, and records a release for whichever
//! service is watching that tag.
//!
//! ## The labels are load-bearing
//!
//! containerd's garbage collector walks from image records through
//! `containerd.io/gc.ref.content.*` labels. A manifest committed
//! without them is a manifest whose layers are unreferenced — and the
//! next collection deletes the image out from under a service that is
//! about to be deployed from it.

use containerd_client::types::Descriptor;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::{RestError, RestResult};

use wabot::sqlite::rusqlite::OptionalExtension;

use crate::platform::{images, releases, services};
use crate::runtime::content;

use super::blobs::{connect, internal, read_body};
use super::{Pusher, RegistryState};

/// What this node calls an image somebody pushed.
///
/// The repository as the client named it, under the node's own host —
/// so `ctr images ls` shows something a person could paste back into a
/// `docker pull`.
fn image_name(host: &str, name: &str, reference: &str) -> String {
    if reference.starts_with("sha256:") {
        format!("{host}/{name}@{reference}")
    } else {
        format!("{host}/{name}:{reference}")
    }
}

/// Just enough of a manifest to know what it points at.
///
/// Deliberately not a full OCI model: what this needs is the config
/// and the layers, and a struct that ignores everything else keeps
/// working when a client sends a field this version never heard of.
#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    config: Option<Blob>,
    #[serde(default)]
    layers: Vec<Blob>,
    /// Present on an index — a manifest of manifests, one per
    /// platform.
    #[serde(default)]
    manifests: Vec<Blob>,
}

#[derive(Debug, Deserialize)]
struct Blob {
    digest: String,
}

impl Manifest {
    /// Every blob this manifest depends on.
    fn referenced(&self) -> Vec<String> {
        self.config
            .iter()
            .map(|blob| blob.digest.clone())
            .chain(self.layers.iter().map(|blob| blob.digest.clone()))
            .chain(self.manifests.iter().map(|blob| blob.digest.clone()))
            .collect()
    }
}

pub async fn head(state: &RegistryState, name: &str, reference: &str) -> RestResult<Response> {
    match lookup(state, name, reference).await? {
        Some((digest, size, media_type)) => Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, media_type)
            .header(header::CONTENT_LENGTH, size)
            .header("docker-content-digest", digest)
            .body(Body::empty())
            .expect("a constant response is well-formed")),
        None => Ok(missing()),
    }
}

pub async fn get(state: &RegistryState, name: &str, reference: &str) -> RestResult<Response> {
    let Some((digest, _, media_type)) = lookup(state, name, reference).await? else {
        return Ok(missing());
    };

    let client = connect().await?;
    let descriptor = Descriptor {
        digest: digest.clone(),
        ..Default::default()
    };
    let bytes = content::read(&client, &descriptor)
        .await
        .map_err(internal)?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, media_type)
        .header(header::CONTENT_LENGTH, bytes.len())
        .header("docker-content-digest", digest)
        .body(Body::from(bytes))
        .expect("a constant response is well-formed"))
}

/// `PUT /v2/<name>/manifests/<reference>` — the last request of a push.
pub async fn put(
    state: &RegistryState,
    pusher: &Pusher,
    name: &str,
    reference: &str,
    request: Request,
) -> RestResult<Response> {
    let media_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/vnd.oci.image.manifest.v1+json")
        .to_string();

    let body = read_body(request).await?;
    let digest = format!("sha256:{:x}", Sha256::digest(&body));

    let manifest: Manifest = serde_json::from_slice(&body).map_err(|error| RestError::Client {
        status: 400,
        message: format!("that is not a manifest: {error}"),
    })?;

    // The manifest is a blob like any other, and it has to carry the
    // labels that keep what it points at alive.
    let client = connect().await?;
    let size = body.len() as i64;
    content::commit(
        &client,
        &format!("manifest-{digest}"),
        &digest,
        size,
        body,
        content::gc_labels(&manifest.referenced()),
    )
    .await
    .map_err(internal)?;

    // Then the image record, which is what makes it an image rather
    // than bytes: the runtime resolves images by name through this.
    let host = state.host.clone();
    let full = image_name(&host, name, reference);
    create_image(&client, &full, &digest, size, &media_type).await?;

    // And the same manifest under its digest. A deployment runs the
    // digest, never the tag — so without this record the runtime
    // cannot resolve what it is asked to run, and tries to *pull* it
    // from a registry named after the first path segment. That failure
    // reads as a DNS error about a host nobody configured.
    let pinned = image_name(&host, name, &digest);
    create_image(&client, &pinned, &digest, size, &media_type).await?;

    // And the registry's own record of the tag, which is what a client
    // is told about when it asks whether this push is needed.
    if !reference.starts_with("sha256:") {
        remember(state, name, reference, &digest, size, &media_type).await?;
    }

    // Unpacked before anything is deployed from it. A push leaves
    // blobs; a container needs a snapshot, and the failure without
    // this reads as a corrupt image rather than a missing step.
    if let Err(error) = crate::runtime::images::unpack(&client, &pinned).await {
        // Not fatal to the push: the bytes are stored and correct, and
        // a deployment can unpack later. Worth saying, because a
        // deployment that then fails would otherwise be the first
        // anybody hears of it.
        tracing::warn!(image = %pinned, %error, "received but not unpacked");
    }

    let released = release(state, pusher, name, reference, &digest).await?;

    tracing::info!(image = %full, %digest, released, "received an image");
    Ok(Response::builder()
        .status(StatusCode::CREATED)
        .header(header::LOCATION, format!("/v2/{name}/manifests/{digest}"))
        .header("docker-content-digest", digest)
        .header(header::CONTENT_LENGTH, "0")
        .body(Body::empty())
        .expect("a constant response is well-formed"))
}

/// Record the push against whichever services watch this tag, and
/// deploy the ones that asked to be deployed.
///
/// Returns whether anything was recorded — a push to a repository no
/// service uses is stored and does nothing, which is what somebody
/// pushing a base image expects.
async fn release(
    state: &RegistryState,
    pusher: &Pusher,
    name: &str,
    reference: &str,
    digest: &str,
) -> RestResult<bool> {
    // A digest reference is not a tag: it names bytes, and nothing
    // watches it.
    if reference.starts_with("sha256:") {
        return Ok(false);
    }

    // With the host, because this is what will be deployed: a
    // reference with no host is one containerd reads as `<first
    // segment>` being a registry to dial.
    let pushed = format!("{}/{name}:{reference}", state.host);
    let services = services::all(&state.database, Some(&pusher.project.id)).await?;
    let mut recorded = false;

    for service in services {
        let Some(service_reference) = images::Reference::parse(&service.image) else {
            continue;
        };
        // Compared by name rather than by full reference: the client
        // pushed to a host it dialled, and the service may name the
        // same repository with or without one.
        if service_reference.name() != name {
            continue;
        }
        if images::tracked_tag(&service.image, service.track_tag.as_deref()).as_deref()
            != Some(reference)
        {
            continue;
        }

        let release = releases::record(
            &state.database,
            &service.id,
            &pushed,
            digest,
            releases::Source::Push,
        )
        .await?;
        recorded = true;

        if service.auto_deploy {
            // Deployed here rather than by a background sweep: the
            // push is the event, and a CI run that succeeds should
            // mean the thing is out — not that something will notice
            // eventually.
            state.deployer.deploy_release(&service, &release).await;
        }
    }

    Ok(recorded)
}

/// Create or update the image record containerd resolves by name.
///
/// One line, because the same thing is done when a backup is restored:
/// `runtime::images::record` is the implementation and this is the
/// registry's door to it. It was written twice, and the copy here was
/// the one carrying the GC-root label that stops the manifest being
/// collected — a second copy is a second chance to leave that out.
async fn create_image(
    client: &crate::runtime::client::Containerd,
    name: &str,
    digest: &str,
    size: i64,
    media_type: &str,
) -> RestResult<()> {
    crate::runtime::images::record(client, name, digest, size, media_type)
        .await
        .map_err(internal)
}

/// What this reference resolves to, if this node has it.
///
/// A tag is answered from the registry's **own** index, not from
/// containerd's image records. Those are shared with everything else
/// on the node — `ctr images tag` writes one, a pull writes one — so
/// answering from them tells a client "already have it" about images
/// nobody pushed. The client then skips the upload, the push reports
/// success, no release is recorded, and nothing says why. Found
/// exactly that way, on the node, with a push that did nothing.
///
/// A digest needs no index at all: the content store is addressed by
/// it, and sharing that store is the whole design.
async fn lookup(
    state: &RegistryState,
    name: &str,
    reference: &str,
) -> RestResult<Option<(String, i64, String)>> {
    if reference.starts_with("sha256:") {
        let client = connect().await?;
        return Ok(content::exists(&client, reference)
            .await
            .map_err(internal)?
            .map(|size| {
                (
                    reference.to_string(),
                    size,
                    "application/vnd.oci.image.manifest.v1+json".to_string(),
                )
            }));
    }

    let (repository, tag) = (name.to_string(), reference.to_string());
    state
        .database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"digest\", \"size\", \"media_type\" FROM registry_tag \
                     WHERE \"repository\" = ?1 AND \"tag\" = ?2",
                    (repository, tag),
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
        })
        .await
        .map_err(internal)
}

/// Record that this repository's tag now names this manifest.
async fn remember(
    state: &RegistryState,
    name: &str,
    reference: &str,
    digest: &str,
    size: i64,
    media_type: &str,
) -> RestResult<()> {
    let (repository, tag) = (name.to_string(), reference.to_string());
    let (digest, media_type) = (digest.to_string(), media_type.to_string());

    state
        .database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO registry_tag \
                   (\"repository\", \"tag\", \"digest\", \"media_type\", \"size\", \
                    \"updated_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT (\"repository\", \"tag\") DO UPDATE SET \
                   \"digest\" = excluded.\"digest\", \
                   \"media_type\" = excluded.\"media_type\", \
                   \"size\" = excluded.\"size\", \
                   \"updated_at\" = excluded.\"updated_at\"",
                (
                    repository,
                    tag,
                    digest,
                    media_type,
                    size,
                    crate::platform::now_ms(),
                ),
            )?;
            Ok(())
        })
        .await
        .map_err(internal)
}

fn missing() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("docker-distribution-api-version", "registry/2.0")
        .body(Body::empty())
        .expect("a constant response is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_is_named_the_way_somebody_would_pull_it() {
        assert_eq!(
            image_name("node.example", "demo/api", "v1"),
            "node.example/demo/api:v1"
        );
        // A digest is joined with `@`, not `:` — `name:sha256:…` is
        // not a reference anything can resolve.
        assert_eq!(
            image_name("node.example", "demo/api", "sha256:abc"),
            "node.example/demo/api@sha256:abc"
        );
    }

    /// Every blob a manifest points at has to be labelled, or the
    /// collector takes the layers and leaves the manifest.
    #[test]
    fn a_manifest_reports_its_config_and_layers() {
        let manifest: Manifest = serde_json::from_str(
            r#"{
              "schemaVersion": 2,
              "mediaType": "application/vnd.oci.image.manifest.v1+json",
              "config": {"digest": "sha256:cfg", "size": 12},
              "layers": [
                {"digest": "sha256:one", "size": 1},
                {"digest": "sha256:two", "size": 2}
              ]
            }"#,
        )
        .expect("parsed");

        assert_eq!(
            manifest.referenced(),
            ["sha256:cfg", "sha256:one", "sha256:two"]
        );
    }

    /// An index points at manifests rather than layers, and those need
    /// keeping alive just the same.
    #[test]
    fn an_index_reports_the_manifests_under_it() {
        let index: Manifest = serde_json::from_str(
            r#"{
              "schemaVersion": 2,
              "mediaType": "application/vnd.oci.image.index.v1+json",
              "manifests": [{"digest": "sha256:amd64"}, {"digest": "sha256:arm64"}]
            }"#,
        )
        .expect("parsed");

        assert_eq!(index.referenced(), ["sha256:amd64", "sha256:arm64"]);
    }

    /// A client is free to send fields this version has never heard
    /// of, and a push must not fail because of one.
    #[test]
    fn unknown_fields_are_ignored() {
        let manifest: Manifest = serde_json::from_str(
            r#"{"config": {"digest": "sha256:cfg"}, "layers": [], "subject": {"whatever": 1}}"#,
        )
        .expect("parsed");
        assert_eq!(manifest.referenced(), ["sha256:cfg"]);
    }

    #[test]
    fn the_labels_name_every_dependency() {
        let labels = content::gc_labels(&["sha256:a".into(), "sha256:b".into()]);
        let values: std::collections::BTreeSet<&String> = labels.values().collect();

        assert_eq!(labels.len(), 2);
        assert!(values.contains(&"sha256:a".to_string()));
        assert!(values.contains(&"sha256:b".to_string()));
    }
}
