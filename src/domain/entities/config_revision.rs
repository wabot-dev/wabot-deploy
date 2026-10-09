//! One saved version of a service's environment.

use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Revision {
    pub id: String,
    pub service_id: String,
    pub env: BTreeMap<String, String>,
    pub changed_by: Option<String>,
    /// `edit` or `revert` — what somebody was doing.
    pub reason: String,
    pub created_at: i64,
}
