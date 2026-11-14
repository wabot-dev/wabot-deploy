use crate::domain::values::allowlist::Allowlist;

/// Who may connect to a service, as a whole list. Empty admits nobody.
pub struct SetServiceAllowlistRequest {
    pub allowlist: Allowlist,
}
