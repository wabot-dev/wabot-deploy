//! Where a project's containers may connect outside its own network.
//!
//! Empty lets nothing out, as an empty allowlist lets nobody in: anywhere
//! is `0.0.0.0/0`, written down by somebody. IPv4 only, because a container
//! has no IPv6 address, and a line that could never match would read as a
//! rule that does something.
//!
//! A name is what a line allows the project to resolve, and the addresses
//! it resolves to are what the kernel opens — `docs/egress.md`, "Names".

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

use ipnet::{Ipv4Net, Ipv6Net};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Range(Ipv4Net),
    /// `under` is a `*.` line: every name below this one, not this one.
    Name {
        name: String,
        under: bool,
    },
}

impl Target {
    pub fn written(&self) -> String {
        match self {
            Target::Range(range) if range.prefix_len() == 32 => range.addr().to_string(),
            Target::Range(range) => range.to_string(),
            Target::Name { name, under: true } => format!("*.{name}"),
            Target::Name { name, under: false } => name.clone(),
        }
    }

    fn holds_name(&self, asked: &str) -> bool {
        match self {
            Target::Range(_) => false,
            Target::Name { name, under: false } => asked == name,
            Target::Name { name, under: true } => asked
                .strip_suffix(name.as_str())
                .is_some_and(|before| before.len() > 1 && before.ends_with('.')),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub target: Target,
    pub port: Option<u16>,
    pub note: Option<String>,
}

impl Destination {
    pub fn written(&self) -> String {
        match self.port {
            Some(port) => format!("{}:{port}", self.target.written()),
            None => self.target.written(),
        }
    }

    fn is_anywhere(&self) -> bool {
        self.port.is_none() && self.is_every_address()
    }

    fn is_every_address(&self) -> bool {
        matches!(self.target, Target::Range(range) if range.prefix_len() == 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Destinations(Vec<Destination>);

impl Destinations {
    pub fn new(entries: Vec<Destination>) -> Self {
        let mut kept: Vec<Destination> = Vec::with_capacity(entries.len());
        for entry in entries {
            let target = match entry.target {
                Target::Range(range) => Target::Range(range.trunc()),
                name => name,
            };
            if !kept
                .iter()
                .any(|held| held.target == target && held.port == entry.port)
            {
                kept.push(Destination { target, ..entry });
            }
        }
        Self(kept)
    }

    /// One per line: an address, a range or a name, a port after a colon if
    /// it is one port, then a note.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (written, note) = match line.split_once(char::is_whitespace) {
                Some((written, note)) => (written, Some(note.trim().to_string())),
                None => (line, None),
            };
            let (target, port) =
                destination(written).map_err(|reason| format!("line {}: {reason}", number + 1))?;
            entries.push(Destination {
                target,
                port,
                note: note.filter(|note| !note.is_empty()),
            });
        }
        Ok(Self::new(entries))
    }

    pub fn text(&self) -> String {
        self.0
            .iter()
            .map(|entry| match &entry.note {
                Some(note) => format!("{} {note}", entry.written()),
                None => entry.written(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn is_anywhere(&self) -> bool {
        self.0.iter().any(Destination::is_anywhere)
    }

    /// Every range on the list, with its port.
    pub fn ranges(&self) -> Vec<(Ipv4Net, Option<u16>)> {
        self.0
            .iter()
            .filter_map(|entry| match entry.target {
                Target::Range(range) => Some((range, entry.port)),
                Target::Name { .. } => None,
            })
            .collect()
    }

    /// The ports the lines naming `asked` allow, `None` for every port.
    /// Empty when no line names it.
    pub fn ports_for(&self, asked: &str) -> Vec<Option<u16>> {
        let asked = normalised(asked);
        let mut ports: Vec<Option<u16>> = self
            .0
            .iter()
            .filter(|entry| entry.target.holds_name(&asked))
            .map(|entry| entry.port)
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    /// Whether a name no line names may still be asked about upstream.
    ///
    /// Asking is itself what leaks it: the question reaches whoever answers
    /// for the name, whatever comes back. So only a project that may already
    /// reach every address, on some port, has its other names asked about,
    /// because it could send the same thing over that port anyway.
    pub fn asks_about_any_name(&self) -> bool {
        self.0.iter().any(Destination::is_every_address)
    }
}

impl fmt::Display for Destinations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text())
    }
}

/// What is not the internet: the ranges a project's own node reaches
/// directly whatever its exits, and an exit never dials for somebody else.
/// An exit that did would hand its own network, its link-local metadata
/// service included, to every project leaving through it.
pub const NOT_THE_INTERNET: [&str; 11] = [
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "224.0.0.0/4",
    "240.0.0.0/4",
];

pub fn is_internet(address: Ipv4Addr) -> bool {
    !NOT_THE_INTERNET.iter().any(|range| {
        range
            .parse::<Ipv4Net>()
            .is_ok_and(|range| range.contains(&address))
    })
}

/// A name as a query or an answer spells it, compared as the list stores
/// it.
pub fn normalised(name: &str) -> String {
    name.trim_end_matches('.').to_ascii_lowercase()
}

fn destination(written: &str) -> Result<(Target, Option<u16>), String> {
    if written.parse::<Ipv6Addr>().is_ok() || written.parse::<Ipv6Net>().is_ok() {
        return Err(format!(
            "“{written}” is IPv6, and a container has no IPv6 address to reach it from"
        ));
    }
    let (target, port) = match written.split_once(':') {
        Some((target, port)) => {
            let port = port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| format!("“{port}” is not a port"))?;
            (target, Some(port))
        }
        None => (written, None),
    };
    let looks_like_an_address = target
        .chars()
        .all(|character| character.is_ascii_digit() || character == '.' || character == '/');
    let target = match looks_like_an_address {
        true => match target.contains('/') {
            true => target.parse::<Ipv4Net>().ok(),
            false => target.parse::<Ipv4Addr>().ok().map(Ipv4Net::from),
        }
        .map(Target::Range),
        false => name(target),
    }
    .ok_or_else(|| {
        format!(
            "“{target}” is not an address, a range like 203.0.113.0/24, \
             or a name like api.example.com"
        )
    })?;
    Ok((target, port))
}

fn name(written: &str) -> Option<Target> {
    let lowered = normalised(written);
    let (under, name) = match lowered.strip_prefix("*.") {
        Some(name) => (true, name.to_string()),
        None => (false, lowered),
    };
    let labels: Vec<&str> = name.split('.').collect();
    let well_formed = name.len() <= 253
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        });
    (well_formed && (under || labels.len() > 1)).then_some(Target::Name { name, under })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_names_a_range_or_an_address_a_port_and_a_note() {
        let list = Destinations::parse(
            "203.0.113.0/24 the office\n198.51.100.7:443\n\n# a comment\n192.0.2.0/28:5432 payments db",
        )
        .expect("parse");

        let written: Vec<(String, Option<u16>, Option<&str>)> = list
            .0
            .iter()
            .map(|entry| (entry.target.written(), entry.port, entry.note.as_deref()))
            .collect();
        assert_eq!(
            written,
            vec![
                ("203.0.113.0/24".into(), None, Some("the office")),
                ("198.51.100.7".into(), Some(443), None),
                ("192.0.2.0/28".into(), Some(5432), Some("payments db")),
            ]
        );
    }

    #[test]
    fn empty_lets_nothing_out_and_anywhere_is_one_written_line() {
        let empty = Destinations::parse("\n  \n").expect("parse");
        assert!(empty.is_empty());
        assert!(!empty.is_anywhere());

        assert!(Destinations::parse("0.0.0.0/0")
            .expect("parse")
            .is_anywhere());
    }

    #[test]
    fn anywhere_on_one_port_is_not_anywhere() {
        let list = Destinations::parse("0.0.0.0/0:443").expect("parse");
        assert!(!list.is_anywhere());
        assert!(!list.is_empty());
    }

    #[test]
    fn an_ipv6_line_is_refused_because_no_container_could_reach_it() {
        for line in ["::/0", "2001:db8::1", "2001:db8::/32"] {
            let refused = Destinations::parse(line).expect_err(line);
            assert!(refused.contains("IPv6"), "{line}: {refused}");
        }
    }

    #[test]
    fn a_range_written_from_inside_itself_is_the_range_it_names_kept_once() {
        let list = Destinations::parse("10.0.0.5/24\n10.0.0.0/24\n10.0.0.0/24:80").expect("parse");
        assert_eq!(list.text(), "10.0.0.0/24\n10.0.0.0/24:80");
    }

    #[test]
    fn a_line_that_is_not_a_destination_says_which_line_and_why() {
        let not_an_address = Destinations::parse("10.0.0.1\n10.0.0.300").expect_err("refused");
        assert!(not_an_address.starts_with("line 2:"), "{not_an_address}");
        for written in [
            "localhost",
            "api..example.com",
            "-api.example.com",
            "api_x.example.com",
            "*",
            "a.*.example.com",
        ] {
            let refused = Destinations::parse(written).expect_err(written);
            assert!(
                refused.contains("is not an address"),
                "{written}: {refused}"
            );
        }

        for port in ["10.0.0.1:0", "10.0.0.1:70000", "10.0.0.1:https"] {
            let refused = Destinations::parse(port).expect_err(port);
            assert!(refused.contains("is not a port"), "{port}: {refused}");
        }
    }

    #[test]
    fn what_a_list_says_reads_back_as_the_same_list() {
        let list =
            Destinations::parse("203.0.113.5:443 stripe\n0.0.0.0/0\n192.0.2.0/24").expect("parse");
        assert_eq!(Destinations::parse(&list.text()).expect("read back"), list);
    }

    #[test]
    fn a_name_allows_itself_and_a_starred_name_allows_what_is_below_it() {
        let list =
            Destinations::parse("API.Stripe.com\n*.files.example.com:443 uploads").expect("parse");
        assert_eq!(
            list.text(),
            "api.stripe.com\n*.files.example.com:443 uploads"
        );

        assert_eq!(list.ports_for("api.stripe.com."), vec![None]);
        assert_eq!(list.ports_for("API.STRIPE.COM"), vec![None]);
        assert!(list.ports_for("stripe.com").is_empty());
        assert!(list.ports_for("evil-api.stripe.com").is_empty());
        assert_eq!(list.ports_for("eu.files.example.com"), vec![Some(443)]);
        assert_eq!(list.ports_for("a.b.files.example.com"), vec![Some(443)]);
        assert!(list.ports_for("files.example.com").is_empty());
        assert!(list.ports_for("xfiles.example.com").is_empty());
    }

    #[test]
    fn a_name_on_two_lines_is_allowed_every_port_either_allows() {
        let list = Destinations::parse(
            "api.example.com:443\n*.example.com:8443\napi.example.com:443 again",
        )
        .expect("parse");
        assert_eq!(
            list.ports_for("api.example.com"),
            vec![Some(443), Some(8443)]
        );
    }

    /// The question is the leak: a name somebody encodes data into reaches
    /// the server answering for it whatever the answer is, so a project that
    /// may not reach every address is never asked about upstream for a name
    /// its list does not hold.
    #[test]
    fn only_a_project_that_reaches_every_address_has_other_names_asked_about() {
        for (list, asks) in [
            ("0.0.0.0/0", true),
            ("0.0.0.0/0:443", true),
            ("10.0.0.0/8\napi.example.com", false),
            ("", false),
        ] {
            assert_eq!(
                Destinations::parse(list)
                    .expect("parse")
                    .asks_about_any_name(),
                asks,
                "{list:?}"
            );
        }
    }

    #[test]
    fn the_internet_is_what_no_private_or_special_range_holds() {
        for (address, internet) in [
            ("140.82.114.6", true),
            ("1.1.1.1", true),
            ("10.42.1.5", false),
            ("192.168.104.1", false),
            ("172.20.0.1", false),
            ("169.254.169.254", false),
            ("100.64.0.1", false),
            ("127.0.0.1", false),
            ("224.0.0.251", false),
        ] {
            assert_eq!(
                is_internet(address.parse().expect("an address")),
                internet,
                "{address}"
            );
        }
    }
}
