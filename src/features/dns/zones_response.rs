use serde::Serialize;

use crate::infra::dns::server::Listening;

#[derive(Debug, Clone, Serialize)]
pub struct ZonesResponse {
    pub zones: Vec<ZoneRow>,
    pub provides_dns: bool,
    pub addressed: bool,
    pub listening: Vec<Listening>,
    /// What this node's operator typed, for the form; empty is its domain.
    pub declared: Vec<String>,
    pub answers_as: Vec<OwnName>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnName {
    pub name: String,
    pub addresses: Vec<String>,
    pub here: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ZoneRow {
    pub id: String,
    pub name: String,
    pub owner: ZoneOwner,
    pub records: usize,
    pub answered_here: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ZoneOwner {
    ThisNode,
    Organisation(String),
    SentBy(String),
}

impl ZoneOwner {
    /// Names rather than ids: the page says whose a zone is, and an id is
    /// nobody's name.
    pub fn of(
        zone: &crate::domain::entities::dns_zone::DnsZone,
        organisations: &std::collections::BTreeMap<String, String>,
        nodes: &std::collections::BTreeMap<String, String>,
    ) -> Self {
        if let Some(authority) = &zone.authority_id {
            return ZoneOwner::SentBy(
                nodes
                    .get(authority)
                    .cloned()
                    .unwrap_or_else(|| authority.clone()),
            );
        }
        match &zone.tenant_id {
            Some(tenant) => ZoneOwner::Organisation(
                organisations
                    .get(tenant)
                    .cloned()
                    .unwrap_or_else(|| tenant.clone()),
            ),
            None => ZoneOwner::ThisNode,
        }
    }
}
