use serde::Serialize;

/// A name that answers with a permanent redirect to one of its service's
/// own hostnames, keeping the path and the query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Redirect {
    pub hostname: String,
    pub port_id: String,
    pub service_id: String,
    /// The hostname of `port_id`, read with it, so it is never a name the
    /// port no longer has.
    pub target: String,
}
