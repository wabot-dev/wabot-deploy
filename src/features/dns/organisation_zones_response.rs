use serde::Serialize;

use crate::features::dns::zone_response::NameServer;
use crate::features::dns::zones_response::ZoneRow;

/// An organisation's domains, and the name servers a new one gets: what
/// they will be told to set at their registrar before they have added it.
#[derive(Debug, Clone, Serialize)]
pub struct OrganisationZonesResponse {
    pub zones: Vec<ZoneRow>,
    pub name_servers: Vec<NameServer>,
}
