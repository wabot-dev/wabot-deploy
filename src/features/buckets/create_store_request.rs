use serde::{Deserialize, Serialize};
use wabot::prelude::*;

use crate::domain::values::allowlist::Allowlist;

/// Make a store of buckets.
///
/// `memory` is a preset, as a database's is: the number also sizes what
/// Garage keeps in memory. `disk` is a size, because it is what the buckets'
/// maximums are added up against.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct CreateStoreRequest {
    #[min_length(1)]
    pub project: String,
    #[min_length(1)]
    #[max_length(60)]
    pub name: String,
    /// Empty is the name the node derives for it.
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub memory: String,
    #[serde(default)]
    pub disk: String,
    /// The node it runs on, by id; empty for this one.
    #[serde(default)]
    pub on: String,
    #[serde(skip)]
    pub allowlist: Allowlist,
}
