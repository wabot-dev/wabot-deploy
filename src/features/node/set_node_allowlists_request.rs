use std::net::IpAddr;

use crate::domain::values::allowlist::Allowlist;

/// Who may connect to this node's console, the registry under its name and
/// its SSH server, and where the change came from.
pub struct SetNodeAllowlistsRequest {
    pub console: Allowlist,
    pub registry: Allowlist,
    /// Empty closes SSH.
    pub ssh: Allowlist,
    /// The address the request arrived from, which the console's list has
    /// to keep letting in.
    pub client: Option<IpAddr>,
}
