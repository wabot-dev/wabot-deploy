use serde::{Deserialize, Serialize};
use wabot::prelude::*;

use crate::domain::values::allowlist::Allowlist;

/// Make a managed database.
///
/// `memory` is a preset and not a free size: the number also sets the
/// engine's arithmetic, so 64 MB of ceiling with the stock
/// `shared_buffers` of 128 MB is a container killed before it starts.
/// Which presets exist is `policies::presets`, and the refusal is the
/// command's.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct CreateDatabaseRequest {
    #[min_length(1)]
    pub project: String,
    #[min_length(1)]
    #[max_length(60)]
    pub name: String,
    #[serde(default)]
    pub engine: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub memory: String,
    /// `None` is the node's own image for the engine and version, and
    /// `Some("")` is not: somebody who asked for their own image and
    /// left the field empty made a mistake, and taking the default would
    /// hide it.
    #[serde(default)]
    pub image: Option<String>,
    /// The node its primary runs on, by id; empty for this one.
    #[serde(default)]
    pub on: String,
    /// Who may connect to it once it is published. Empty admits nobody.
    #[serde(skip)]
    pub allowlist: Allowlist,
}
