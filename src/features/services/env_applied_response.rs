use serde::{Deserialize, Serialize};

/// The environment was written, and the service wants redeploying.
///
/// The id and not the service: what the caller does with this is queue a
/// deployment, and handing it the entity would put a model with rules
/// into a place whose only job is to name a row.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EnvAppliedResponse {
    pub service_id: String,
    /// The release that is deployed, so the redeployment keeps the image
    /// it is on rather than moving to the newest one.
    pub release_id: Option<String>,
}
