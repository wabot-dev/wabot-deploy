use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Make a managed database.
///
/// `memory` is a preset and not a free size: the number also sets the
/// engine's arithmetic, so 64 MB of ceiling with the stock
/// `shared_buffers` of 128 MB is a container killed before it starts.
/// Which presets exist is `policies::presets`, and the refusal is the
/// command's.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct CreateDatabase {
    #[min_length(1)]
    pub project: String,
    #[min_length(1)]
    #[max_length(60)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub memory: String,
}
