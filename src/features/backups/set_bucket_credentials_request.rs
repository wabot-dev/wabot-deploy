use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// The key this node signs its uploads with.
///
/// It lands in the **config file, never the database** — a database is
/// what a backup copies, and a backup carrying the key to its own bucket
/// is a key in every copy.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct SetBucketCredentialsRequest {
    #[serde(default)]
    pub access_key_id: String,
    /// Empty keeps what is stored: a page that showed the secret back to
    /// be re-typed would be a page that shows the secret.
    #[serde(default)]
    pub secret_access_key: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub endpoint: String,
}
