use std::collections::BTreeMap;

use crate::domain::values::slug::Slug;

pub struct CreateEnvGroupRequest {
    pub name: Slug,
    pub env: BTreeMap<String, String>,
}
