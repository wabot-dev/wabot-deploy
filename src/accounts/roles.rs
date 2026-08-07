//! Who may do what.
//!
//! ## Two levels, because there are two kinds of question
//!
//! "May this person create a project, invite somebody, read the node's
//! memory" is about the node. "May this person deploy *here*" is about
//! one project, and the answer differs per project — which is the
//! whole reason projects exist.
//!
//! ## An unknown role is the smallest role
//!
//! Every parse falls back to the least privilege it could mean. A row
//! written by a version that knows a role this one does not must not
//! grant more than this version understands — the failure of the other
//! direction is silent and total.
//!
//! ## Decisions are values, not conditions
//!
//! Every check returns an [`Access`], and every caller asks it a
//! question by name: `may_deploy()`, not `role == Deployer || role ==
//! Owner || account.is_admin()`. The comparison spelled out at each
//! call site is how one of thirty of them ends up missing a clause.

use serde::{Deserialize, Serialize};

/// What somebody is on this node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    /// Everything, everywhere, including the people page and every
    /// project without being a member of it.
    Admin,
    /// An ordinary person: their own projects, and the ones they were
    /// added to.
    Member,
}

impl NodeRole {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeRole::Admin => "admin",
            NodeRole::Member => "member",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text {
            "admin" => NodeRole::Admin,
            // Including anything this version has never heard of.
            _ => NodeRole::Member,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NodeRole::Admin => "Administrator",
            NodeRole::Member => "Member",
        }
    }
}

/// What somebody is inside one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectRole {
    /// Read. Nothing else.
    Viewer,
    /// Services and ports: create, deploy, stop, remove.
    Deployer,
    /// The project itself: rename, delete, and who else is in it.
    Owner,
}

impl ProjectRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectRole::Owner => "owner",
            ProjectRole::Deployer => "deployer",
            ProjectRole::Viewer => "viewer",
        }
    }

    /// The order is `Viewer < Deployer < Owner`, so a comparison means
    /// "at least this much" — which is what every check wants and what
    /// a set of booleans could not express.
    pub fn parse(text: &str) -> Self {
        match text {
            "owner" => ProjectRole::Owner,
            "deployer" => ProjectRole::Deployer,
            _ => ProjectRole::Viewer,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProjectRole::Owner => "Owner",
            ProjectRole::Deployer => "Deployer",
            ProjectRole::Viewer => "Viewer",
        }
    }

    /// The roles somebody can be given, most capable first.
    pub const ALL: [ProjectRole; 3] = [
        ProjectRole::Owner,
        ProjectRole::Deployer,
        ProjectRole::Viewer,
    ];
}

/// What one person may do about one project.
///
/// Built by the layer that knows both — see `platform::access` — and
/// handed to a handler that only has to ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    /// Set when the account is an administrator, which outranks
    /// membership entirely.
    pub admin: bool,
    /// Their role in this project, if they are in it.
    pub role: Option<ProjectRole>,
}

impl Access {
    /// An administrator, who is in every project without being a
    /// member of any.
    pub const ADMIN: Access = Access {
        admin: true,
        role: None,
    };

    /// Somebody with no relationship to this project at all.
    pub const NONE: Access = Access {
        admin: false,
        role: None,
    };

    pub fn member(role: ProjectRole) -> Self {
        Self {
            admin: false,
            role: Some(role),
        }
    }

    /// May they see it exists?
    ///
    /// Everything else is built on this: a project somebody cannot see
    /// must answer as though it were not there, or the list of
    /// projects on a node is readable by asking for each name in turn.
    pub fn may_read(&self) -> bool {
        self.admin || self.role.is_some()
    }

    /// May they change what runs — services, ports, deployments?
    pub fn may_deploy(&self) -> bool {
        self.at_least(ProjectRole::Deployer)
    }

    /// May they rename or delete the project, and manage who is in it?
    pub fn may_administer(&self) -> bool {
        self.at_least(ProjectRole::Owner)
    }

    fn at_least(&self, role: ProjectRole) -> bool {
        self.admin || self.role.is_some_and(|held| held >= role)
    }

    /// How to describe this to the person it applies to.
    pub fn label(&self) -> &'static str {
        match (self.admin, self.role) {
            (true, _) => "Administrator",
            (false, Some(role)) => role.label(),
            (false, None) => "No access",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_survive_the_round_trip_through_storage() {
        for role in [NodeRole::Admin, NodeRole::Member] {
            assert_eq!(NodeRole::parse(role.as_str()), role);
        }
        for role in ProjectRole::ALL {
            assert_eq!(ProjectRole::parse(role.as_str()), role);
        }
    }

    /// The failure that matters: a role this version does not know
    /// must not be read as more than it is. Guessing high once is a
    /// silent privilege grant; guessing low is a visible refusal
    /// somebody reports.
    #[test]
    fn an_unknown_role_is_the_smallest_one() {
        assert_eq!(NodeRole::parse("superuser"), NodeRole::Member);
        assert_eq!(NodeRole::parse(""), NodeRole::Member);
        assert_eq!(ProjectRole::parse("maintainer"), ProjectRole::Viewer);
        assert_eq!(
            ProjectRole::parse("OWNER"),
            ProjectRole::Viewer,
            "case matters"
        );
    }

    #[test]
    fn a_viewer_may_look_and_nothing_else() {
        let access = Access::member(ProjectRole::Viewer);
        assert!(access.may_read());
        assert!(!access.may_deploy());
        assert!(!access.may_administer());
    }

    #[test]
    fn a_deployer_may_change_what_runs_but_not_the_project() {
        let access = Access::member(ProjectRole::Deployer);
        assert!(access.may_read());
        assert!(access.may_deploy());
        assert!(!access.may_administer(), "not theirs to delete");
    }

    #[test]
    fn an_owner_may_do_everything_in_their_project() {
        let access = Access::member(ProjectRole::Owner);
        assert!(access.may_read() && access.may_deploy() && access.may_administer());
    }

    /// An administrator is in every project without being a member of
    /// any. A node whose operator has to add themselves to a project
    /// before they can fix it is one that locks them out of the thing
    /// they operate.
    #[test]
    fn an_administrator_needs_no_membership() {
        let access = Access::ADMIN;
        assert_eq!(access.role, None);
        assert!(access.may_read() && access.may_deploy() && access.may_administer());
    }

    /// A stranger must not be able to tell a project they cannot see
    /// from one that does not exist.
    #[test]
    fn somebody_with_no_membership_may_do_nothing() {
        let access = Access::NONE;
        assert!(!access.may_read());
        assert!(!access.may_deploy());
        assert!(!access.may_administer());
        assert_eq!(access.label(), "No access");
    }

    /// The ordering is what lets a check say "at least this much"
    /// instead of listing the roles that qualify — which is how one
    /// call site ends up missing one.
    #[test]
    fn the_roles_are_ordered_by_how_much_they_allow() {
        assert!(ProjectRole::Owner > ProjectRole::Deployer);
        assert!(ProjectRole::Deployer > ProjectRole::Viewer);
    }
}
