use crate::domain::values::allowlist::Allowlist;

/// Who may connect to an organisation's console, as a whole list. Empty
/// admits nobody.
pub struct SetOrganisationAllowlistRequest {
    pub organisation: String,
    pub allowlist: Allowlist,
}
