use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Open a port on a service, and optionally give it a name.
///
/// Three of the four fields only mean anything together, which is why
/// they travel as one shape: `hostname` is empty unless `https`, and
/// `publish` is about the node's own address rather than the name. The
/// refusals that pair them — *serving over HTTPS needs a hostname*, and
/// what DNS says about that name — are the command's, because each has a
/// sentence a person can act on.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct AddServicePortRequest {
    pub container_port: u16,
    #[serde(default)]
    pub publish: bool,
    /// Empty unless HTTPS was asked for.
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub https: bool,
}
