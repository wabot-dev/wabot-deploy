use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct EnvGroupSavedResponse {
    pub redeploy: Vec<Redeploy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Redeploy {
    pub service_id: String,
    /// So the redeployment keeps the image it is on rather than moving to
    /// the newest one.
    pub release_id: Option<String>,
}
