//! Pulling images, and reading what they say about themselves.
//!
//! ## The Transfer service, not a hand-rolled pull
//!
//! `Transfer` does resolve, fetch, store **and unpack** server-side, in
//! one call. The alternative is the Diff service per layer — prepare a
//! snapshot, apply the layer, commit, compute the chain ID, set the
//! `containerd.io/uncompressed` label — which is a few hundred lines
//! and every one of them a chance to compute a chain ID that containerd
//! will not recognise as the image's.
//!
//! The unpack is the part that matters. Without it the blobs are in the
//! content store and there is no snapshot, so no task can start.
//!
//! ## The image's own configuration is not optional
//!
//! An image says what to run, as whom, where, and with what
//! environment. A container built without reading it starts with an
//! empty command and fails inside the shim, where the error says
//! nothing useful.

use containerd_client::services::v1::images_client::ImagesClient;
use containerd_client::services::v1::transfer_client::TransferClient;
use containerd_client::services::v1::{GetImageRequest, TransferOptions, TransferRequest};
use containerd_client::to_any;
use containerd_client::types::transfer::{ImageStore, OciRegistry, UnpackConfiguration};
use containerd_client::types::Platform;

use super::client::{ClientError, ClientResult, Containerd};

/// What an image tells the node about how to run it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageConfig {
    /// `Entrypoint` followed by `Cmd`, which is what actually runs.
    pub command: Vec<String>,
    pub env: Vec<String>,
    pub working_dir: Option<String>,
    /// `User`, as the image wrote it: `""`, `"1000"`, `"1000:1000"`, or
    /// a name this node cannot resolve without the image's `/etc/passwd`.
    pub user: Option<String>,
    /// Ports the image declares. Advisory — a hint about which port to
    /// route to when the operator did not say.
    pub exposed_ports: Vec<u16>,
}

/// This machine's platform, in the spelling image manifests use.
pub fn platform() -> Platform {
    Platform {
        os: "linux".to_string(),
        // The release/manifest name, not Rust's: `amd64`, not `x86_64`.
        architecture: match std::env::consts::ARCH {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            other => other,
        }
        .to_string(),
        variant: String::new(),
        os_version: String::new(),
    }
}

/// Pull `reference` and unpack it, so a task can be started from it.
///
/// Idempotent in the way that matters: layers already in the content
/// store are not downloaded again — containerd short-circuits on
/// `content.Exists` — so a re-pull costs a few manifest requests.
pub async fn pull(client: &Containerd, reference: &str) -> ClientResult<()> {
    let platform = platform();

    let source = OciRegistry {
        reference: reference.to_string(),
        resolver: Default::default(),
    };
    let destination = ImageStore {
        name: reference.to_string(),
        platforms: vec![platform.clone()],
        // The unpack. Without this the blobs land and no snapshot is
        // created, and the container fails to start with an error about
        // a missing rootfs rather than about a missing unpack.
        unpacks: vec![UnpackConfiguration {
            platform: Some(platform),
            ..Default::default()
        }],
        ..Default::default()
    };

    tracing::info!(image = reference, "pulling");
    TransferClient::new(client.channel())
        .transfer(client.request(TransferRequest {
            source: Some(to_any(&source)),
            destination: Some(to_any(&destination)),
            options: Some(TransferOptions::default()),
        }))
        .await
        .map_err(|source| ClientError::Call {
            call: "Transfer",
            source,
        })?;

    tracing::info!(image = reference, "pulled");
    Ok(())
}

/// Is this image already here?
pub async fn exists(client: &Containerd, reference: &str) -> ClientResult<bool> {
    match ImagesClient::new(client.channel())
        .get(client.request(GetImageRequest {
            name: reference.to_string(),
        }))
        .await
    {
        Ok(_) => Ok(true),
        Err(status) if status.code() == tonic::Code::NotFound => Ok(false),
        Err(source) => Err(ClientError::Call {
            call: "Images.Get",
            source,
        }),
    }
}

/// Pull only if it is not already here.
pub async fn ensure(client: &Containerd, reference: &str) -> ClientResult<bool> {
    if exists(client, reference).await? {
        return Ok(false);
    }
    pull(client, reference).await?;
    Ok(true)
}

/// Read the image's configuration out of the content store.
///
/// Two hops: the image record points at a manifest, the manifest points
/// at a config blob, and the config blob is the JSON with `Env`,
/// `Entrypoint` and the rest. A multi-platform image points at an index
/// first, so there can be three.
pub async fn config(client: &Containerd, reference: &str) -> ClientResult<ImageConfig> {
    let target = image_target(client, reference).await?;
    let config_descriptor = resolve_config_descriptor(client, &target).await?;
    let bytes = super::content::read(client, &config_descriptor).await?;

    let config: oci_spec::image::ImageConfiguration = serde_json::from_slice(&bytes)
        .map_err(|error| ClientError::Other(format!("unreadable image config: {error}")))?;

    let Some(inner) = config.config() else {
        // Legal per the spec and useless in practice: nothing to run.
        return Ok(ImageConfig::default());
    };

    let mut command: Vec<String> = inner.entrypoint().clone().unwrap_or_default();
    command.extend(inner.cmd().clone().unwrap_or_default());

    Ok(ImageConfig {
        command,
        env: inner.env().clone().unwrap_or_default(),
        working_dir: inner.working_dir().clone(),
        user: inner.user().clone().filter(|user| !user.is_empty()),
        exposed_ports: inner
            .exposed_ports()
            .as_ref()
            .map(|ports| ports.iter().filter_map(|port| parse_port(port)).collect())
            .unwrap_or_default(),
    })
}

/// The image's uncompressed layer digests, in order.
///
/// Separate from [`config`] because they answer a different question:
/// the config says how to run the image, these say which snapshot its
/// rootfs is. Both come out of the same blob, and reading it twice
/// costs one small gRPC call against a local socket.
pub async fn diff_ids(client: &Containerd, reference: &str) -> ClientResult<Vec<String>> {
    let target = image_target(client, reference).await?;
    let descriptor = resolve_config_descriptor(client, &target).await?;
    let bytes = super::content::read(client, &descriptor).await?;

    let config: oci_spec::image::ImageConfiguration = serde_json::from_slice(&bytes)
        .map_err(|error| ClientError::Other(format!("unreadable image config: {error}")))?;

    Ok(config.rootfs().diff_ids().clone())
}

/// `"8080/tcp"` → `8080`. UDP is dropped: this node routes HTTP.
fn parse_port(declared: &str) -> Option<u16> {
    let (port, protocol) = match declared.split_once('/') {
        Some((port, protocol)) => (port, protocol),
        None => (declared, "tcp"),
    };
    if protocol != "tcp" {
        return None;
    }
    port.parse().ok()
}

/// The descriptor an image record points at.
async fn image_target(
    client: &Containerd,
    reference: &str,
) -> ClientResult<containerd_client::types::Descriptor> {
    let image = ImagesClient::new(client.channel())
        .get(client.request(GetImageRequest {
            name: reference.to_string(),
        }))
        .await
        .map_err(|source| ClientError::Call {
            call: "Images.Get",
            source,
        })?
        .into_inner()
        .image
        .ok_or_else(|| ClientError::Other(format!("{reference} has no target")))?;

    image
        .target
        .ok_or_else(|| ClientError::Other(format!("{reference} has no target descriptor")))
}

/// Walk from whatever the image points at down to its config blob.
///
/// An index has to be resolved to the manifest for *this* platform
/// first. Taking the first entry would work on a single-arch image and
/// silently run an arm64 rootfs on x86_64 for a multi-arch one.
async fn resolve_config_descriptor(
    client: &Containerd,
    target: &containerd_client::types::Descriptor,
) -> ClientResult<containerd_client::types::Descriptor> {
    let bytes = super::content::read(client, target).await?;
    let wanted = platform();

    match target.media_type.as_str() {
        // An index: pick the manifest for this platform, then recurse.
        media if is_index(media) => {
            let index: oci_spec::image::ImageIndex = serde_json::from_slice(&bytes)
                .map_err(|error| ClientError::Other(format!("unreadable image index: {error}")))?;

            let chosen = index
                .manifests()
                .iter()
                .find(|descriptor| {
                    descriptor.platform().as_ref().is_some_and(|platform| {
                        platform.architecture().to_string() == wanted.architecture
                            && platform.os().to_string() == wanted.os
                    })
                })
                .ok_or_else(|| {
                    ClientError::Other(format!(
                        "the image has no {}/{} manifest",
                        wanted.os, wanted.architecture
                    ))
                })?;

            let manifest = containerd_client::types::Descriptor {
                media_type: chosen.media_type().to_string(),
                digest: chosen.digest().to_string(),
                // oci-spec says u64, containerd's proto says i64.
                size: chosen.size() as i64,
                annotations: Default::default(),
            };
            Box::pin(resolve_config_descriptor(client, &manifest)).await
        }
        // A manifest: its `config` is what we came for.
        _ => {
            let manifest: oci_spec::image::ImageManifest =
                serde_json::from_slice(&bytes).map_err(|error| {
                    ClientError::Other(format!("unreadable image manifest: {error}"))
                })?;
            let config = manifest.config();
            Ok(containerd_client::types::Descriptor {
                media_type: config.media_type().to_string(),
                digest: config.digest().to_string(),
                size: config.size() as i64,
                annotations: Default::default(),
            })
        }
    }
}

/// Both spellings, because an image built by Docker uses one and an OCI
/// one uses the other, and a node that handled only its favourite would
/// fail on half of Docker Hub.
fn is_index(media_type: &str) -> bool {
    media_type == "application/vnd.oci.image.index.v1+json"
        || media_type == "application/vnd.docker.distribution.manifest.list.v2+json"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest spelling, not Rust's. `x86_64` appears in no image
    /// index, and asking for it finds nothing.
    #[test]
    fn the_platform_uses_manifest_names() {
        let platform = platform();
        assert_eq!(platform.os, "linux");
        assert!(
            matches!(platform.architecture.as_str(), "amd64" | "arm64"),
            "got {}",
            platform.architecture
        );
    }

    #[test]
    fn both_index_spellings_are_recognised() {
        assert!(is_index("application/vnd.oci.image.index.v1+json"));
        assert!(is_index(
            "application/vnd.docker.distribution.manifest.list.v2+json"
        ));
        assert!(!is_index("application/vnd.oci.image.manifest.v1+json"));
        assert!(!is_index("application/vnd.oci.image.config.v1+json"));
    }

    #[test]
    fn exposed_ports_drop_what_cannot_be_routed() {
        assert_eq!(parse_port("8080/tcp"), Some(8080));
        assert_eq!(parse_port("80"), Some(80), "a bare port is tcp");
        assert_eq!(parse_port("53/udp"), None, "this node routes HTTP");
        assert_eq!(parse_port("nonsense"), None);
    }

    /// An image with `Entrypoint` *and* `Cmd` runs both, concatenated —
    /// entrypoint first. Using one or the other gives a container that
    /// starts and does the wrong thing.
    #[test]
    fn entrypoint_and_cmd_concatenate() {
        let json = serde_json::json!({
            "architecture": "amd64",
            "os": "linux",
            "rootfs": { "type": "layers", "diff_ids": [] },
            "config": {
                "Entrypoint": ["/bin/sh", "-c"],
                "Cmd": ["echo hi"],
                "Env": ["PATH=/usr/bin"],
                "WorkingDir": "/app",
                "User": "1000",
                "ExposedPorts": { "8080/tcp": {} }
            }
        });
        let parsed: oci_spec::image::ImageConfiguration =
            serde_json::from_value(json).expect("valid config");
        let inner = parsed.config().clone().expect("has config");

        let mut command: Vec<String> = inner.entrypoint().clone().unwrap_or_default();
        command.extend(inner.cmd().clone().unwrap_or_default());
        assert_eq!(command, vec!["/bin/sh", "-c", "echo hi"]);
        assert_eq!(inner.working_dir().clone().as_deref(), Some("/app"));
        assert_eq!(inner.user().clone().as_deref(), Some("1000"));
    }
}
