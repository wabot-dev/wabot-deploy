//! What the node deploys: projects, and the services under them.
//!
//! ## One containerd namespace, labels for ownership
//!
//! A namespace per project was the obvious shape and is the wrong one.
//! containerd namespaces scope image *content* as well as metadata, so
//! two projects running the same base image would keep two copies of
//! every layer — and the embedded registry that shares the content
//! store, which is the whole storage design, would have nothing to
//! share. What a namespace does not give is network, resource or
//! security isolation.
//!
//! So: one namespace, containers named `<project>--<service>`, and
//! "one project cannot see another's" enforced by this node's API,
//! which is where an authorization rule belongs anyway.
//!
//! Network isolation is a real gap and a separate piece of work — a
//! CNI bridge per project — which lands after services work end to
//! end. Until then containers share the host's network and the node
//! allocates ports.

pub mod projects;
pub mod registry_credentials;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("storage: {0}")]
    Storage(#[from] wabot::sqlite::SqliteError),
    #[error("{0}")]
    Refused(String),
}

pub type PlatformResult<T> = Result<T, PlatformError>;

/// See the note on `AccountError`'s conversion: a refusal is the
/// caller's to fix and says so; a storage failure is ours and does not
/// describe itself over HTTP.
impl From<PlatformError> for wabot::rest::RestError {
    fn from(error: PlatformError) -> Self {
        match error {
            PlatformError::Refused(message) => wabot::rest::RestError::Client {
                status: 400,
                message,
            },
            other => {
                tracing::error!(error = %other, "platform operation failed");
                wabot::rest::RestError::Internal("platform operation failed".into())
            }
        }
    }
}
