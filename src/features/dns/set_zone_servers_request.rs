use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// The whole set each time: a checkbox that is off sends nothing, so a
/// form that sent only the changes could never take a node away.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct SetZoneServersRequest {
    #[min_length(1)]
    pub zone: String,
    #[serde(default)]
    pub servers: Vec<String>,
}
