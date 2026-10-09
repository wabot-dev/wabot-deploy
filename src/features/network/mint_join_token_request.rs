use serde::{Deserialize, Serialize};
use wabot::prelude::*;

use crate::network::capability::Capability;

/// Mint a token another node can enrol with.
///
/// **A join states its terms**, and the two lists are them: what this
/// node *requires* of the joiner, and what it *offers* to do for them.
/// They are separate because they bind opposite machines — a required
/// capability is a grant the joiner must write, an offered one is a
/// grant this node writes at the callback.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct MintJoinTokenRequest {
    /// A note, so the nodes page names the machine rather than an id.
    #[max_length(60)]
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub requires: Vec<Capability>,
    #[serde(default)]
    pub offers: Vec<Capability>,
}
