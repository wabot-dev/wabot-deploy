use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// The three ceilings, as the form spells them.
///
/// Text rather than numbers because what a person types is a size —
/// `512MB`, `1.5`, `20GB` — and `presets::parse_size` is where the
/// grammar and its refusal live. Typed here, serde would refuse first
/// and say only *invalid digit*.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct SetServiceResources {
    #[serde(default)]
    pub memory: String,
    #[serde(default)]
    pub cpu: String,
    #[serde(default)]
    pub disk: String,
}
