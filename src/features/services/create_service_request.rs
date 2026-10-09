use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Make a service in a project.
///
/// **The controller builds this, and it arrives validated.** What the
/// rules here cover is *shape* — a name that is a name, a project that
/// was named at all. Whether the project exists, whether this actor may
/// deploy in it and whether the node owns services are the use case's
/// and the entity's, and none of them can be asked of a form.
#[derive(Debug, Clone, Deserialize, Serialize, Validate)]
pub struct CreateServiceRequest {
    /// The project's slug, as the URL spells it.
    #[min_length(1)]
    pub project: String,
    #[min_length(1)]
    #[max_length(60)]
    pub name: String,
    /// Empty is a state, not a mistake: a service can wait for its first
    /// push. Whether it is a reference at all is
    /// `values::ImageReference`.
    pub image: String,
    /// `KEY=value` lines, as the form carries them.
    #[serde(default)]
    pub env: String,
}
