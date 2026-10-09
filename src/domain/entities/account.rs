use serde::{Deserialize, Serialize};

use crate::domain::values::roles::NodeRole;

/// Serializable because this is also what travels through `Auth`: the
/// console's claims are the account, and a second struct of the same
/// shape is a second place for it to drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub username: String,
    /// What they are on the node. Travels in the session claims, so a
    /// handler knows without a second query — and a role changed while
    /// somebody is signed in takes effect on their next request, since
    /// the middleware reads the row every time.
    pub role: NodeRole,
    /// Which theme they read in. Travels with the account so the
    /// choice follows the person rather than the browser.
    pub theme: crate::domain::values::theme::Theme,
    /// Which language they read in. Beside the theme and for the same
    /// reason: a preference that does not follow somebody between
    /// machines is one they set again on each.
    pub language: crate::domain::values::language::Language,
    /// Which tenant they belong to. `None` is this node's own operator.
    ///
    /// **A scope, not a role.** `role` says how much somebody may do and
    /// this says over what, and keeping them apart is what lets
    /// `is_admin` gain one clause instead of forty-three call sites
    /// gaining a third case to remember. See migration `0054`.
    ///
    /// It travels in the session claims like the role, so the frontier
    /// is known before any handler runs, and a person moved between
    /// scopes is moved on their next click.
    pub tenant_id: Option<String>,
}

impl Account {
    /// Whether they administer **this machine** — the network page, the
    /// node's settings, updates, and every project on it.
    ///
    /// **A tenant's administrator is not one of these**, and the clause
    /// saying so lives here rather than at the checks, because there are
    /// forty-three of those and the one that forgot would be a customer
    /// with the node's own settings page. `role` means "administrator of
    /// my own scope"; this asks whether that scope is the machine.
    pub fn is_admin(&self) -> bool {
        self.role == NodeRole::Admin && self.tenant_id.is_none()
    }

    /// Whether they administer whatever scope they are in.
    ///
    /// What a tenant's people page and quota card ask. Distinct from
    /// [`is_admin`](Self::is_admin) by exactly one thing, and the two
    /// must never be spelled the same way at a call site.
    pub fn administers_scope(&self) -> bool {
        self.role == NodeRole::Admin
    }

    /// Whether this account belongs to the scope a request arrived in.
    ///
    /// **The `Host` decides the scope before anything reads a session**,
    /// so this is the check that keeps the two in agreement: a tenant's
    /// cookie presented on the node's own hostname is a valid session
    /// for the wrong frontier. Nothing else it could reach would be
    /// theirs — `access::of` narrows by membership either way — but the
    /// `Host` stops being the boundary the moment one identity can be
    /// used across two, and a boundary that holds only because every
    /// query remembered to narrow is the thirty-comparisons problem
    /// `roles.rs` exists to avoid.
    pub fn belongs_to(&self, scope: Option<&str>) -> bool {
        self.tenant_id.as_deref() == scope
    }
}
