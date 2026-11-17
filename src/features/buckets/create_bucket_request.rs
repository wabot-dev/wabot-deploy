use serde::{Deserialize, Serialize};
use wabot::prelude::*;

#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct CreateBucketRequest {
    #[min_length(1)]
    pub name: String,
    /// A size, like `5 GB`: Garage's quota for it, and its share of the
    /// store's disk.
    #[min_length(1)]
    pub max_size: String,
}
