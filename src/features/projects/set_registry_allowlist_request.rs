use crate::domain::values::allowlist::Allowlist;

/// Who may reach a project's registry on its own name, as a whole list.
/// Empty admits nobody.
pub struct SetRegistryAllowlistRequest {
    pub allowlist: Allowlist,
}
