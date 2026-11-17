use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Which store comes back, from which backup, and what the copy is called
/// and answers to. An empty moment is the newest backup there is; an empty
/// name is the original's with `-restored`; an empty hostname is derived,
/// as a new store's is.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct RestoreStoreRequest {
    pub project: String,
    pub store: String,
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub into: String,
    #[serde(default)]
    pub hostname: String,
}
