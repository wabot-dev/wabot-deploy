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
        // The deploy unpacks too — `images::ensure` asks whether an image
        // is *usable* rather than merely present, which is what a pushed
        // one is not. Before that, this warning was the only trace and
        // what somebody saw was a deploy refusing with `no unpacked
        // layer`: true, and nothing they could act on. Reported by Jorge,
        // whose first push to his own service could not be deployed.
        tracing::warn!(
            image = %pinned, %error,
            "received but not unpacked — the deploy will try again, and if it fails the reason \
             lands on the service"
        );
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

    // **A service's own repository is `<project>/<service>`**, and a push
    // to it is a release for that service whatever its image field says.
    //
    // This used to require the field to name the pushed repository — and
    // the console's push card tells somebody to push to exactly this
    // reference while offering no way to change that field, because there
    // is no form for it anywhere. So the console's own instruction
    // produced a push the node accepted into the registry and then
    // ignored: the image stored, the release absent, and nothing saying
    // why. Reported by Jorge, who was right that editing a field should
    // not be part of pushing.
    //
    // Sound rather than merely convenient: the token guard already
    // refuses a repository that is not this project's, a slug is unique
    // within a project, and deploying a release runs `release.pinned()` —
    // the digest — so the image field was never what a release ran.
    let by_slug = name
        .split_once('/')
        .filter(|(project, _)| *project == pusher.project.slug)
        .map(|(_, service)| service.to_string());

    for service in services {
        // Either the service's own repository, or an image field that
        // names the pushed one. The second is kept because a service
        // deliberately pointed at `<host>/<something-else>` is a thing
        // somebody can have set up, and this is not the release that
        // takes it away.
        let named_here = images::Reference::parse(&service.image)
            .is_some_and(|reference| reference.name() == name);
        if by_slug.as_deref() != Some(service.slug.as_str()) && !named_here {
            continue;
        }
        // **The watched tag decides what deploys, not what is recorded.**
        //
        // A push to this service's own repository is a version of this
        // service whatever tag it carries, so it belongs in the list
        // either way and `Deploy` beside it is how somebody rolls to it
        // deliberately. This used to `continue` here, so a push to any
        // other tag vanished: accepted by the registry, stored on the
        // disk, absent from the page, and nothing saying why.
        //
        // Two questions were on one line and only one of them is
        // dangerous — recording is a row, deploying replaces what is
        // running. Reported by Jorge, who wanted to see the image he had
        // sent.
        // Through `watches`, which is the only thing that answers this —
        // `*` means every tag, and a comparison against one name cannot
        // express that.
        let watched = images::watches(&service.image, service.track_tag.as_deref(), reference);

        // **The service now runs what was pushed, so its row says so.**
        //
        // Without this the field keeps naming wherever the service was
        // created from — Docker Hub, usually — and the page goes on
        // telling somebody a push here can never reach it, immediately
        // after one did. Deploying a release pins the digest either way,
        // so this changes nothing about what runs; it changes what the
        // console can say truthfully, and it is what makes the *next*
        // push match by name as well as by slug.
        //
        // Only when it differs, so an ordinary push writes nothing.
        // Adopted only for the watched tag: a push to another tag is a
        // version somebody may want *later*, and repointing the service at
        // it would change what an ordinary deploy pulls — the one thing a
        // push nobody is watching must not do.
        if watched && service.image != pushed {
            if let Err(error) = services::set_image(&state.database, &service.id, &pushed).await {
                // Reported and not fatal: the release below is the thing
                // that was asked for, and refusing it because a field
                // could not be tidied would lose the push.
                tracing::warn!(%error, service = %service.slug, "could not adopt the pushed image");
            }
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

        // Both conditions, and they mean different things: `auto_deploy`
        // is "go out on a push", `watched` is "on *this* tag". A push to
        // any other tag is now recorded and waits to be deployed by hand.
        if watched && service.auto_deploy {
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
    /// A push to a service's own repository is a release for it, whatever
    /// its image field says — including when it says nothing.
    ///
    /// This required the field to name the pushed repository, and the
    /// console's push card tells somebody to push to exactly this
    /// reference while offering no way to change that field: there was no
    /// form for it anywhere. So the console's own instruction produced a
    /// push the node stored and then ignored — the release absent, and
    /// nothing saying why. Reported by Jorge, who was right that editing a
    /// field should not be part of pushing.
    #[test]
    fn a_services_own_repository_is_the_one_it_watches() {
        // The rule, as the matcher applies it: the first segment is the
        // project and the second is the service's slug.
        for (pushed, project, slug, matches) in [
            ("wabot/nginx-2", "wabot", "nginx-2", true),
            // Another project's repository is not this service's, and the
            // token guard refuses it before this is reached anyway.
            ("other/nginx-2", "wabot", "nginx-2", false),
            // Same project, different service.
            ("wabot/api", "wabot", "nginx-2", false),
            // No slash at all is not a project repository.
            ("nginx-2", "wabot", "nginx-2", false),
        ] {
            let by_slug = pushed
                .split_once('/')
                .filter(|(p, _)| *p == project)
                .map(|(_, service)| service.to_string());
            assert_eq!(
                by_slug.as_deref() == Some(slug),
                matches,
                "{pushed} against {project}/{slug}"
            );
        }
    }

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
