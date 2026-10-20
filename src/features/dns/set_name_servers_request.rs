use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// The whole list each time, in order: the first is the primary in every
/// zone's SOA, and an empty list is the node's own domain again.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct SetNameServersRequest {
    #[serde(default)]
    pub names: Vec<String>,
}
