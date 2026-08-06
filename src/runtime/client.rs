//! Talking to containerd.
//!
//! `containerd-client` is generated bindings and nothing else — there
//! is no `Pull`, no `NewContainer`, no spec generation. What Go's
//! `containerd.Client` does for you is written here instead.
//!
//! ## The namespace is not optional
//!
//! containerd rejects almost every call without a `containerd-namespace`
//! header. One namespace for everything this node does, because content
//! is not visible across namespaces — and the registry sharing
//! containerd's content store is the whole storage story.
//!
//! ## crun is selected per container
//!
//! Not by configuration. `/etc/containerd/config.toml` sets
//! `BinaryName` under the CRI plugin, which is the Kubernetes-facing
//! API this node does not use; the native API takes runtime options in
//! `Containers.Create`. Verified by watching `ctr run` look for a runc
//! that is not installed.

use std::time::Duration;

use tonic::transport::{Channel, Endpoint, Uri};

/// One namespace for everything. Named for the product rather than
/// `default`, so `ctr -n` output says whose containers these are.
pub const NAMESPACE: &str = "wabot";

/// The shim. Misleadingly named: it drives any runtime with a
/// runc-compatible CLI, and [`RuncOptions::binary_name`] picks which.
pub const RUNTIME: &str = "io.containerd.runc.v2";

/// The type URL containerd expects for runc-shim options.
///
/// Not a guess — it is the protobuf message's fully-qualified name, and
/// the shim looks it up by exactly this string. A typo here produces a
/// container that starts under the *default* runtime, silently, which
/// is a bad way to discover it.
const RUNC_OPTIONS_TYPE_URL: &str = "types.containerd.io/containerd.runc.v1.Options";

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("containerd is not reachable at {socket}: {source}")]
    Connect {
        socket: String,
        #[source]
        source: tonic::transport::Error,
    },
    #[error("containerd refused {call}: {source}")]
    Call {
        call: &'static str,
        #[source]
        source: tonic::Status,
    },
    #[error("{0}")]
    Other(String),
}

pub type ClientResult<T> = Result<T, ClientError>;

/// The runc shim's options, which `containerd-client` does not generate.
///
/// The proto is vendored in that crate but left out of its build list,
/// so this is a hand-written `prost` message against
/// `api/types/runc/options/oci.proto`. Only the fields this node sets
/// are declared — prost encodes by field number, so the rest being
/// absent is exactly the same as them being default.
///
/// **The numbers are the contract.** `binary_name` is 6 and
/// `systemd_cgroup` is 9 in containerd's proto; getting one wrong sends
/// a value the shim reads as a different setting.
#[derive(Clone, PartialEq, prost::Message)]
pub struct RuncOptions {
    /// Field 6. The runtime binary — this is how crun gets chosen.
    #[prost(string, tag = "6")]
    pub binary_name: String,
    /// Field 9. Without it, memory limits and OOM accounting are wrong
    /// in ways that only surface when a container is killed and the
    /// reason is misreported.
    #[prost(bool, tag = "9")]
    pub systemd_cgroup: bool,
}

impl RuncOptions {
    pub fn crun() -> Self {
        Self {
            binary_name: crate::bootstrap::runtime::CRUN_PATH.to_string(),
            systemd_cgroup: true,
        }
    }

    /// Wrapped as the `Any` that `Container.runtime.options` takes.
    pub fn to_any(&self) -> prost_types::Any {
        prost_types::Any {
            type_url: RUNC_OPTIONS_TYPE_URL.to_string(),
            value: prost::Message::encode_to_vec(self),
        }
    }
}

/// A connection to containerd.
///
/// Cheap to clone — tonic's `Channel` is a handle over a shared
/// connection pool, so cloning shares the socket rather than opening
/// another.
#[derive(Clone)]
pub struct Containerd {
    channel: Channel,
    socket: String,
}

impl std::fmt::Debug for Containerd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The channel has no useful representation; the socket is the
        // only part anyone debugging wants to see.
        f.debug_struct("Containerd")
            .field("socket", &self.socket)
            .finish_non_exhaustive()
    }
}

impl Containerd {
    /// Connect to the default socket.
    pub async fn connect() -> ClientResult<Self> {
        Self::connect_to(crate::bootstrap::runtime::SOCKET).await
    }

    pub async fn connect_to(socket: &str) -> ClientResult<Self> {
        let path = socket.to_string();
        // The URI is a placeholder tonic requires and never dials: the
        // connector below ignores it and opens the unix socket. Without
        // a syntactically valid URI, `Endpoint` refuses to build.
        let channel = Endpoint::try_from("http://[::]:50051")
            .map_err(|source| ClientError::Connect {
                socket: path.clone(),
                source,
            })?
            .connect_timeout(Duration::from_secs(5))
            .connect_with_connector(tower::service_fn({
                let path = path.clone();
                move |_: Uri| {
                    let path = path.clone();
                    async move {
                        let stream = tokio::net::UnixStream::connect(path).await?;
                        Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                    }
                }
            }))
            .await
            .map_err(|source| ClientError::Connect {
                socket: path.clone(),
                source,
            })?;

        Ok(Self {
            channel,
            socket: path,
        })
    }

    pub fn socket(&self) -> &str {
        &self.socket
    }

    /// A request carrying the namespace header containerd requires.
    ///
    /// Every call goes through this. containerd answers `NamespaceRequired`
    /// otherwise, and the error does not say which header is missing.
    pub fn request<T>(&self, message: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(message);
        request.metadata_mut().insert(
            "containerd-namespace",
            NAMESPACE.parse().expect("the namespace is a valid header"),
        );
        request
    }

    pub fn channel(&self) -> Channel {
        self.channel.clone()
    }

    /// containerd's version — the cheapest proof the connection works.
    pub async fn version(&self) -> ClientResult<String> {
        use containerd_client::services::v1::version_client::VersionClient;

        let response = VersionClient::new(self.channel())
            .version(self.request(()))
            .await
            .map_err(|source| ClientError::Call {
                call: "Version",
                source,
            })?
            .into_inner();
        Ok(format!("{} {}", response.version, response.revision))
    }

    /// Make sure the node's namespace exists.
    ///
    /// Idempotent: an `AlreadyExists` is the success case, not an
    /// error. containerd creates a namespace implicitly on some calls
    /// and not others, so doing it once up front removes a class of
    /// "works the second time".
    pub async fn ensure_namespace(&self) -> ClientResult<()> {
        use containerd_client::services::v1::namespaces_client::NamespacesClient;
        use containerd_client::services::v1::{CreateNamespaceRequest, Namespace};

        let result = NamespacesClient::new(self.channel())
            .create(self.request(CreateNamespaceRequest {
                namespace: Some(Namespace {
                    name: NAMESPACE.to_string(),
                    labels: Default::default(),
                }),
            }))
            .await;

        match result {
            Ok(_) => {
                tracing::info!(namespace = NAMESPACE, "created the containerd namespace");
                Ok(())
            }
            // The idempotent case, and the common one after the first
            // boot. Treating it as an error would make every start
            // after the first look like a failure.
            Err(status) if status.code() == tonic::Code::AlreadyExists => Ok(()),
            Err(source) => Err(ClientError::Call {
                call: "Namespaces.Create",
                source,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field numbers are containerd's, and a wrong one sends a
    /// value the shim reads as a different setting. Encoded and checked
    /// on the wire rather than trusted, because the failure mode is a
    /// container that starts under the wrong runtime without saying so.
    #[test]
    fn crun_options_encode_on_the_numbers_containerd_reads() {
        let bytes = prost::Message::encode_to_vec(&RuncOptions::crun());

        // Field 6, wire type 2 (length-delimited) → (6 << 3) | 2 = 0x32.
        assert_eq!(bytes[0], 0x32, "binary_name must be field 6: {bytes:02x?}");
        assert!(
            bytes.windows(4).any(|w| w == b"crun"),
            "the runtime path is in there: {bytes:02x?}"
        );

        // Field 9, wire type 0 (varint) → (9 << 3) | 0 = 0x48, then 1.
        assert!(
            bytes.windows(2).any(|w| w == [0x48, 0x01]),
            "systemd_cgroup must be field 9 and true: {bytes:02x?}"
        );
    }

    #[test]
    fn the_options_carry_the_type_url_the_shim_looks_up() {
        let any = RuncOptions::crun().to_any();
        assert_eq!(
            any.type_url,
            "types.containerd.io/containerd.runc.v1.Options"
        );
        assert!(!any.value.is_empty());
    }

    #[test]
    fn crun_is_the_path_the_installer_used() {
        assert_eq!(
            RuncOptions::crun().binary_name,
            crate::bootstrap::runtime::CRUN_PATH,
            "the runtime the node asks for has to be the one it installed"
        );
    }

    /// An unreachable socket has to fail with something that names it.
    #[tokio::test]
    async fn a_missing_socket_says_where_it_looked() {
        let error = Containerd::connect_to("/nonexistent/containerd.sock")
            .await
            .expect_err("nothing is listening there");
        assert!(
            error.to_string().contains("/nonexistent/containerd.sock"),
            "{error}"
        );
    }
}
