use serde::{Deserialize, Serialize};
use wabot::prelude::*;

#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct CreateBucketKeyRequest {
    #[min_length(1)]
    #[max_length(60)]
    pub name: String,
    /// `read_write` or `read_only`; anything else is read-only.
    #[serde(default)]
    pub access: String,
}
