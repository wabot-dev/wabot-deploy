use serde::{Deserialize, Serialize};
use wabot::prelude::*;

use crate::accounts::roles::{NodeRole, ProjectRole};

/// Mint a link that lets somebody in.
///
/// **One shape for two use cases**, because the ask is the same one: a
/// role, and optionally a project to land in. What differs is who may
/// run it and which frontier the link opens, and neither of those is
/// something the asker says — they are read off the account.
///
/// The two roles carry no rules here. `NodeRole::parse` and
/// `ProjectRole::parse` already answer for anything a form can send,
/// including a word this version has never heard of, so a check would
/// only be able to disagree with them.
#[derive(Debug, Clone, Deserialize, Serialize, Validate)]
pub struct InvitePersonRequest {
    pub role: NodeRole,
    /// Empty means "into no project", which is a whole membership of the
    /// frontier and not a missing field.
    #[serde(default)]
    pub project: String,
    pub project_role: ProjectRole,
}

impl InvitePersonRequest {
    /// The slug, or `None` for the empty and the blank alike — a form
    /// that submits a whitespace-only select is saying the same thing as
    /// one that submits nothing.
    pub fn project(&self) -> Option<&str> {
        Some(self.project.trim()).filter(|slug| !slug.is_empty())
    }
}
