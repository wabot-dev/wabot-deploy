use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Whether anybody may read a bucket's objects without a key, and at which
/// name. An empty name is the one derived under the store's.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct SetBucketPublicRequest {
    pub public: bool,
    #[serde(default)]
    pub hostname: String,
}
