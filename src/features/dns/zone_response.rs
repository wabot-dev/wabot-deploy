use serde::Serialize;

use crate::features::dns::zones_response::ZoneOwner;

#[derive(Debug, Clone, Serialize)]
pub struct ZoneResponse {
    pub id: String,
    pub name: String,
    pub serial: u32,
    pub owner: ZoneOwner,
    pub editable: bool,
    pub answered_here: bool,
    pub provides_dns: bool,
    pub records: Vec<RecordRow>,
    pub name_servers: Vec<NameServer>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordRow {
    pub id: String,
    pub name: String,
    pub fqdn: String,
    pub kind: &'static str,
    pub value: String,
    pub priority: Option<u16>,
    pub ttl: u32,
}

/// What somebody types at their registrar, and where it leads now.
#[derive(Debug, Clone, Serialize)]
pub struct NameServer {
    pub name: String,
    pub addresses: Vec<String>,
}
