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
    pub servers: Vec<ServerChoice>,
    pub delegation: Option<crate::domain::entities::dns_zone::Delegation>,
}

/// A node that could answer for the zone, and what it was last told.
#[derive(Debug, Clone, Serialize)]
pub struct ServerChoice {
    pub node_id: String,
    pub node_name: String,
    pub published: Option<String>,
    pub this_node: bool,
    pub chosen: bool,
    pub agreed: bool,
    pub last: Option<Delivery>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Delivery {
    Waiting,
    Answering,
    Withdrawn,
    Refused(String),
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
