use serde::{Deserialize, Serialize};
use wabot::prelude::*;

use crate::network::capability::Capability;

/// What **this** node agrees to do for another one.
///
/// The whole set each time, agreed and withdrawn together: a form that
/// sent only the ticked boxes could never turn one off, because a
/// checkbox that is off sends nothing at all.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct SetGrantsRequest {
    #[min_length(1)]
    pub node: String,
    #[serde(default)]
    pub agreed: Vec<Capability>,
}
