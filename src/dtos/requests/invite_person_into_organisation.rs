use serde::{Deserialize, Serialize};
use wabot::prelude::*;

use crate::accounts::roles::{NodeRole, ProjectRole};

/// Mint a link into an organisation, from the node's own console.
///
/// Its own request rather than [`super::invite_person::InvitePerson`]
/// with a field added: the organisation is not optional here, and an
/// `Option` that is always `Some` on one path and always `None` on the
/// other is a shape that has to be re-checked at every read.
#[derive(Debug, Clone, Deserialize, Serialize, Validate)]
pub struct InvitePersonIntoOrganisation {
    #[min_length(1)]
    pub organisation: String,
    pub role: NodeRole,
    #[serde(default)]
    pub project: String,
    pub project_role: ProjectRole,
}

impl InvitePersonIntoOrganisation {
    pub fn project(&self) -> Option<&str> {
        Some(self.project.trim()).filter(|slug| !slug.is_empty())
    }
}
