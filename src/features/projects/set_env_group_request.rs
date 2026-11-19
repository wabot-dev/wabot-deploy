use std::collections::BTreeMap;

pub struct SetEnvGroupRequest {
    pub env: BTreeMap<String, String>,
}
