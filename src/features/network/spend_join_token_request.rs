use serde::{Deserialize, Serialize};
use wabot::prelude::*;

use crate::values::capability::Capability;

/// Spend a join token, or read what it asks for first.
///
/// **`agreed` is the order of the whole thing.** A token states what the
/// minting node *requires* and what it *offers*; false means "show me",
/// and nothing is written. Terms shown after committing are a consent
/// screen for a decision already made, which is worse than none because
/// it looks like one.
///
/// `accepted` is what was ticked, and it is only read once `agreed`. It
/// travels as the capabilities themselves rather than as the form's
/// `accept-<name>` keys: a use case that had to know that spelling would
/// be a use case only a browser can call.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct SpendJoinTokenRequest {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub agreed: bool,
    #[serde(default)]
    pub accepted: Vec<Capability>,
}
