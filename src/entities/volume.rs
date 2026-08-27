//! A volume: a directory a container keeps between deployments.
//!
//! The row is the service's, the directory is the *replica's*, and the
//! directory is derived from the container id rather than stored.

use serde::Serialize;

/// One mount point a service keeps its data in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Volume {
    pub id: String,
    pub service_id: String,
    /// A slug. The last component of the directory on the node.
    pub name: String,
    /// Where it appears inside the container.
    pub path: String,
}
