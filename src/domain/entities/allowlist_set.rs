use crate::domain::values::allowlist::Allowlist;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowlistSet {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub list: Allowlist,
}
