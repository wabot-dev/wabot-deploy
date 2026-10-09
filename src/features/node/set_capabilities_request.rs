use serde::{Deserialize, Serialize};
use wabot::prelude::*;

use crate::domain::values::capability::Capability;

/// What this node is willing to be asked for, and whether it owns
/// anything of its own.
///
/// `owns` is not one of the capabilities and is deliberately outside the
/// enum: every variant of `Capability` is something *another* node asks
/// for, and this is a decision nobody asks for — it has no grant and
/// belongs on no terms screen.
///
/// **The whole set travels, not the changes.** A form drawn from part of
/// the list once turned `store` off in silence, and the node stopped
/// being a standby target while refusing the errands for the copies it
/// already held.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct SetCapabilitiesRequest {
    #[serde(default)]
    pub provides: Vec<Capability>,
    #[serde(default)]
    pub owns: bool,
}
