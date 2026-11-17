use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Everything about a bucket that is a setting rather than its name: how much
/// it may hold, which pages may reach it from a browser, and how long what it
/// holds is kept.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct SetBucketRulesRequest {
    #[min_length(1)]
    pub max_size: String,
    /// One origin per line; empty is none.
    #[serde(default)]
    pub cors_origins: String,
    /// Whole days; empty is never.
    #[serde(default)]
    pub expire_days: String,
    #[serde(default)]
    pub abort_uploads_days: String,
}
