//! An organisation: its people, its projects and what it may take.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tenant {
    pub id: String,
    pub name: String,
    pub slug: String,
    /// What its console answers to, and the suffix its service names
    /// hang off. `None` falls back to this node's own domain, which is
    /// what every service did before tenants existed.
    pub domain: Option<String>,
    /// Bytes, millicores, bytes. `None` is no ceiling on that resource.
    pub quota_memory: Option<u64>,
    pub quota_cpu: Option<u32>,
    pub quota_disk: Option<u64>,
    pub created_at: i64,
}
