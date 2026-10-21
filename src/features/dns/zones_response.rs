use serde::Serialize;

use crate::features::dns::zone_response::NameServer;

/// A project's zones, and the name servers a new one would get: what
/// somebody is told to set at their registrar before they have added it.
#[derive(Debug, Clone, Serialize)]
pub struct ZonesResponse {
    pub zones: Vec<ZoneRow>,
    pub name_servers: Vec<NameServer>,
    pub editable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ZoneRow {
    pub name: String,
    pub records: usize,
    pub answered_here: bool,
}
