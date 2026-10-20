use serde::Serialize;

use crate::domain::values::zone_name::ZoneName;

pub const DEFAULT_TTL: u32 = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DnsZone {
    pub id: String,
    pub name: ZoneName,
    /// `None` is the node operator's.
    pub tenant_id: Option<String>,
    /// `None` is administered here; otherwise the node that sent it.
    pub authority_id: Option<String>,
    pub serial: u32,
    pub default_ttl: u32,
    pub delegation: Option<Delegation>,
    /// For a zone sent here, the name servers it was sent with. Empty for
    /// this node's own, whose servers are named when they are served.
    pub received_name_servers: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl DnsZone {
    pub fn is_ours(&self) -> bool {
        self.authority_id.is_none()
    }

    /// The SOA's mailbox, written the way DNS writes one: the first dot is
    /// the `@`.
    pub fn hostmaster(&self) -> String {
        format!("hostmaster.{}", self.name)
    }
}

/// What the world was last seen asking about this zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Delegation {
    pub status: DelegationStatus,
    pub note: String,
    pub checked_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DelegationStatus {
    Here,
    Partly,
    Elsewhere,
    Unknown,
}

impl DelegationStatus {
    pub fn parse(text: &str) -> Self {
        match text {
            "here" => DelegationStatus::Here,
            "partly" => DelegationStatus::Partly,
            "elsewhere" => DelegationStatus::Elsewhere,
            _ => DelegationStatus::Unknown,
        }
    }
}
