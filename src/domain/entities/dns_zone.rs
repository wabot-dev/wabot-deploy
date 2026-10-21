use serde::Serialize;

use crate::domain::values::zone_name::ZoneName;

pub const DEFAULT_TTL: u32 = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DnsZone {
    pub id: String,
    pub name: ZoneName,
    /// `None` on a zone sent here, and on one administered here whose owner
    /// had no project when zones moved into projects (migration 0063).
    pub project_id: Option<String>,
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

/// Where the parent zone sends resolvers for this one, as last asked.
///
/// `delegated` is the name servers the parent lists — what was typed at the
/// registrar — and `reason` is why the question could not be answered, when
/// it could not. Kept as facts rather than a sentence, so the page says it
/// in the reader's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Delegation {
    pub status: DelegationStatus,
    pub delegated: Vec<String>,
    pub reason: Option<String>,
    pub checked_at: i64,
}

impl Delegation {
    /// The parent's answer read against the names that answer now.
    ///
    /// What the parent said is a fact as of when it was asked; what it
    /// *means* depends on this node's side too, which moves without asking
    /// the parent anything — an operator names the node `ns0` and `ns1`, or
    /// a peer stops answering. Kept as a verdict, it would say "points
    /// elsewhere" about a registrar that had just been set right, until the
    /// next check half an hour later.
    pub fn against(&self, answering: &[String]) -> Delegation {
        match self.status {
            DelegationStatus::Here | DelegationStatus::Partly | DelegationStatus::Elsewhere => {
                Delegation {
                    status: verdict(&self.delegated, answering),
                    ..self.clone()
                }
            }
            DelegationStatus::Unregistered | DelegationStatus::Unknown => self.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DelegationStatus {
    /// The parent lists exactly the nodes that answer for it.
    Here,
    /// Some of them, or them and others.
    Partly,
    /// None of them.
    Elsewhere,
    /// The parent has never heard of the domain.
    Unregistered,
    /// The question could not be answered.
    Unknown,
}

impl DelegationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            DelegationStatus::Here => "here",
            DelegationStatus::Partly => "partly",
            DelegationStatus::Elsewhere => "elsewhere",
            DelegationStatus::Unregistered => "unregistered",
            DelegationStatus::Unknown => "unknown",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text {
            "here" => DelegationStatus::Here,
            "partly" => DelegationStatus::Partly,
            "elsewhere" => DelegationStatus::Elsewhere,
            "unregistered" => DelegationStatus::Unregistered,
            _ => DelegationStatus::Unknown,
        }
    }
}

/// What the parent's list means, against the nodes that answer.
///
/// Compared as names, lowercase and without the trailing dot, and as sets:
/// the order a registrar lists them in is not a difference.
pub fn verdict(delegated: &[String], answering: &[String]) -> DelegationStatus {
    let norm = |name: &String| name.trim().trim_end_matches('.').to_ascii_lowercase();
    let delegated: std::collections::BTreeSet<String> = delegated.iter().map(norm).collect();
    let answering: std::collections::BTreeSet<String> = answering.iter().map(norm).collect();
    if delegated.is_empty() {
        return DelegationStatus::Unregistered;
    }
    if delegated == answering {
        return DelegationStatus::Here;
    }
    match delegated.intersection(&answering).next() {
        Some(_) => DelegationStatus::Partly,
        None => DelegationStatus::Elsewhere,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn the_parents_list_is_read_as_a_set_of_names() {
        let ours = names(&["node-0.network.nubebasura.lat", "node-1.tobaw.shop"]);
        assert_eq!(
            verdict(
                &names(&["NODE-1.tobaw.shop.", "node-0.network.nubebasura.lat."]),
                &ours
            ),
            DelegationStatus::Here
        );
        assert_eq!(
            verdict(&names(&["node-0.network.nubebasura.lat"]), &ours),
            DelegationStatus::Partly
        );
        assert_eq!(
            verdict(
                &names(&["node-0.network.nubebasura.lat", "ns1.linode.com"]),
                &ours
            ),
            DelegationStatus::Partly
        );
        assert_eq!(
            verdict(&names(&["ns1.linode.com", "ns2.linode.com"]), &ours),
            DelegationStatus::Elsewhere
        );
        assert_eq!(verdict(&[], &ours), DelegationStatus::Unregistered);
    }

    #[test]
    fn naming_the_node_changes_what_the_last_answer_means_without_asking_again() {
        let seen = Delegation {
            status: DelegationStatus::Elsewhere,
            delegated: names(&["ns0.nubebasura.lat", "ns1.nubebasura.lat"]),
            reason: None,
            checked_at: 7,
        };
        let before = names(&["node-0.network.nubebasura.lat"]);
        let after = names(&["ns0.nubebasura.lat", "ns1.nubebasura.lat"]);
        assert_eq!(seen.against(&before).status, DelegationStatus::Elsewhere);
        assert_eq!(seen.against(&after).status, DelegationStatus::Here);
        assert_eq!(seen.against(&after).checked_at, 7);

        let unasked = Delegation {
            status: DelegationStatus::Unknown,
            delegated: Vec::new(),
            reason: Some("no answer".into()),
            checked_at: 7,
        };
        assert_eq!(unasked.against(&after).status, DelegationStatus::Unknown);
    }
}
