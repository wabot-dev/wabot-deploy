use crate::domain::values::allowlist::Allowlist;
use crate::domain::values::slug::Slug;

pub struct CreateAllowlistSetRequest {
    pub name: Slug,
    pub list: Allowlist,
}
