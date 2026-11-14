//! Who may come in through one of the node's doors.
//!
//! ## Empty is closed
//!
//! An allowlist with nothing on it admits nobody. Decided by Jorge on
//! 2026-11-14: "everyone" is a choice somebody writes down —
//! `0.0.0.0/0` and `::/0` — rather than what a door is when nobody chose,
//! so a door can only be open because somebody opened it.
//!
//! ## A door, not a port
//!
//! The node's console and the registry under its name share a hostname
//! and a port, and one service answers on several names and ports. What
//! somebody wants to restrict is the thing they reach, so that is what a
//! list belongs to.

use std::fmt;
use std::net::IpAddr;

use ipnet::IpNet;

/// What one list guards.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Door {
    /// The node's own console, on its own names.
    Console,
    /// The registry under the node's own names: every `/v2` path there.
    Registry,
    /// An organisation's console, on the name it was given.
    Organisation(String),
    /// A project's registry, on the name it was given.
    ProjectRegistry(String),
    /// Every name a service answers on, and every port it publishes.
    Service(String),
    /// The machine's SSH server. Closed is an empty list.
    Ssh,
}

impl Door {
    /// The two columns a door is stored under.
    pub fn columns(&self) -> (&'static str, &str) {
        match self {
            Door::Console => ("console", ""),
            Door::Registry => ("registry", ""),
            Door::Organisation(id) => ("organisation", id),
            Door::ProjectRegistry(id) => ("project_registry", id),
            Door::Service(id) => ("service", id),
            Door::Ssh => ("ssh", ""),
        }
    }

    /// The other way, for a row. `None` for a kind this version does not
    /// know: a newer one may guard something this one never heard of.
    pub fn from_columns(kind: &str, subject: &str) -> Option<Self> {
        match kind {
            "console" => Some(Door::Console),
            "registry" => Some(Door::Registry),
            "organisation" => Some(Door::Organisation(subject.to_string())),
            "project_registry" => Some(Door::ProjectRegistry(subject.to_string())),
            "service" => Some(Door::Service(subject.to_string())),
            "ssh" => Some(Door::Ssh),
            _ => None,
        }
    }
}

/// One address or range, and what somebody said it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub network: IpNet,
    pub note: Option<String>,
}

impl Entry {
    /// How it is written: an address alone when the range is one address.
    pub fn address(&self) -> String {
        written(&self.network)
    }
}

/// The addresses a door admits. Empty admits nobody.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Allowlist(Vec<Entry>);

impl Allowlist {
    pub fn new(entries: Vec<Entry>) -> Self {
        let mut kept: Vec<Entry> = Vec::with_capacity(entries.len());
        for entry in entries {
            let network = entry.network.trunc();
            if !kept.iter().any(|held| held.network == network) {
                kept.push(Entry {
                    network,
                    note: entry.note,
                });
            }
        }
        Self(kept)
    }

    /// Every address there is, in both families.
    pub fn everyone() -> Self {
        Self::new(
            ["0.0.0.0/0", "::/0"]
                .into_iter()
                .map(|range| Entry {
                    network: range.parse().expect("a range"),
                    note: None,
                })
                .collect(),
        )
    }

    /// One per line: an address or a range, then an optional note.
    ///
    /// `203.0.113.5 office` and `2001:db8::/32` are both lines. A range
    /// written from inside itself, `10.0.0.5/24`, is kept as the range it
    /// names, `10.0.0.0/24`, because that is what it matches anyway.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (address, note) = match line.split_once(char::is_whitespace) {
                Some((address, note)) => (address, Some(note.trim().to_string())),
                None => (line, None),
            };
            let network = parse_network(address).ok_or_else(|| {
                format!(
                    "line {}: “{address}” is not an address or a range like 203.0.113.0/24",
                    number + 1
                )
            })?;
            entries.push(Entry {
                network,
                note: note.filter(|note| !note.is_empty()),
            });
        }
        Ok(Self::new(entries))
    }

    /// What [`parse`](Self::parse) reads back as this list.
    pub fn text(&self) -> String {
        self.0
            .iter()
            .map(|entry| match &entry.note {
                Some(note) => format!("{} {note}", entry.address()),
                None => entry.address(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn entries(&self) -> &[Entry] {
        &self.0
    }

    pub fn networks(&self) -> Vec<IpNet> {
        self.0.iter().map(|entry| entry.network).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn admits(&self, address: IpAddr) -> bool {
        admits(&self.networks(), address)
    }

    /// Whether this admits every address there is.
    pub fn is_everyone(&self) -> bool {
        is_everyone(&self.networks())
    }
}

impl fmt::Display for Allowlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

/// Whether one of these ranges holds `address`.
///
/// An IPv4 client reaching a dual-stack socket arrives as `::ffff:a.b.c.d`,
/// and a list that says `203.0.113.5` means that client.
pub fn admits(networks: &[IpNet], address: IpAddr) -> bool {
    let address = address.to_canonical();
    networks.iter().any(|network| network.contains(&address))
}

/// Whether these ranges hold every address in both families.
pub fn is_everyone(networks: &[IpNet]) -> bool {
    let whole = |network: &IpNet| network.prefix_len() == 0;
    networks.iter().any(|n| whole(n) && n.network().is_ipv4())
        && networks.iter().any(|n| whole(n) && n.network().is_ipv6())
}

/// An address alone when the range is one address, the range otherwise.
pub fn written(network: &IpNet) -> String {
    match network.prefix_len() == network.max_prefix_len() {
        true => network.addr().to_string(),
        false => network.to_string(),
    }
}

fn parse_network(text: &str) -> Option<IpNet> {
    match text.contains('/') {
        true => text.parse::<IpNet>().ok(),
        false => text.parse::<IpAddr>().ok().map(IpNet::from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(text: &str) -> IpAddr {
        text.parse().expect("an address")
    }

    #[test]
    fn an_empty_list_admits_nobody() {
        let list = Allowlist::parse("").expect("parsed");
        assert!(list.is_empty());
        assert!(!list.admits(address("203.0.113.5")));
        assert!(!list.admits(address("::1")));
    }

    #[test]
    fn a_line_is_an_address_or_a_range_and_a_note() {
        let list = Allowlist::parse(
            "203.0.113.5 office\n\n# the VPN\n10.8.0.0/16   vpn of the team \n2001:db8::/32",
        )
        .expect("parsed");

        assert_eq!(
            list.text(),
            "203.0.113.5 office\n10.8.0.0/16 vpn of the team\n2001:db8::/32"
        );
        assert!(list.admits(address("203.0.113.5")));
        assert!(!list.admits(address("203.0.113.6")));
        assert!(list.admits(address("10.8.200.1")));
        assert!(list.admits(address("2001:db8::42")));
    }

    #[test]
    fn a_range_written_from_inside_itself_is_the_range_it_names() {
        let list = Allowlist::parse("10.0.0.5/24\n10.0.0.0/24 again").expect("parsed");
        assert_eq!(list.text(), "10.0.0.0/24", "once, as the range it matches");
    }

    #[test]
    fn a_line_that_is_not_an_address_says_which_line() {
        let refused = Allowlist::parse("203.0.113.5\noffice 203.0.113.6").expect_err("refused");
        assert!(refused.starts_with("line 2:"), "{refused}");
        assert!(refused.contains("office"), "{refused}");
    }

    /// The address an IPv4 client has on a socket that also takes IPv6.
    #[test]
    fn an_ipv4_client_on_a_dual_stack_socket_is_still_itself() {
        let list = Allowlist::parse("203.0.113.5").expect("parsed");
        assert!(list.admits(address("::ffff:203.0.113.5")));
    }

    #[test]
    fn everyone_is_both_families_and_not_one_of_them() {
        assert!(Allowlist::everyone().is_everyone());
        assert!(Allowlist::everyone().admits(address("198.51.100.7")));
        assert!(Allowlist::everyone().admits(address("2001:db8::1")));
        assert!(!Allowlist::parse("0.0.0.0/0").expect("parsed").is_everyone());
    }

    #[test]
    fn a_door_survives_its_columns() {
        for door in [
            Door::Console,
            Door::Registry,
            Door::Organisation("tnt-1".into()),
            Door::ProjectRegistry("prj-1".into()),
            Door::Service("svc-1".into()),
        ] {
            let (kind, subject) = door.columns();
            assert_eq!(Door::from_columns(kind, subject), Some(door.clone()));
        }
        assert_eq!(Door::from_columns("ssh-from-the-future", ""), None);
    }
}
