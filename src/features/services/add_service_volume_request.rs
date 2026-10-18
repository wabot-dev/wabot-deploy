use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Keep a directory of a service's between deployments.
///
/// `name` may be empty: it is taken from the last part of the path, which
/// is what somebody would have typed anyway.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct AddServiceVolumeRequest {
    #[min_length(1)]
    pub path: String,
    #[serde(default)]
    pub name: String,
}
