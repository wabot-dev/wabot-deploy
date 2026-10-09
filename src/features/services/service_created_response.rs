use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Where the browser goes next, which is the only thing the caller
/// needs — not the `Service` row, which has rules of its own.
#[derive(Debug, Clone, Deserialize, Serialize, Validate)]
pub struct ServiceCreatedResponse {
    /// What the deployment this creation queues is about. Carried
    /// because the caller queues it — see `DeployService`, which is a
    /// command with an async door and therefore the console's dispatch.
    pub service_id: String,
    pub project_slug: String,
    pub service_slug: String,
}
