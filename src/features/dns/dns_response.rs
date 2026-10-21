use serde::Serialize;

use crate::infra::dns::server::Listening;

#[derive(Debug, Clone, Serialize)]
pub struct DnsResponse {
    pub provides_dns: bool,
    pub addressed: bool,
    pub answering_any: bool,
    pub listening: Vec<Listening>,
    /// What this node's operator typed, for the form; empty is its domain.
    pub declared: Vec<String>,
    pub answers_as: Vec<OwnName>,
    pub held: Vec<HeldZone>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnName {
    pub name: String,
    pub addresses: Vec<String>,
    pub here: bool,
}

/// A zone this node answers for that no project here holds: one another
/// node sent, or one whose owner had no project when zones moved into them.
#[derive(Debug, Clone, Serialize)]
pub struct HeldZone {
    pub name: String,
    /// The node that sent it, by name; `None` is one left in no project.
    pub sent_by: Option<String>,
    pub records: usize,
    pub answered_here: bool,
}
