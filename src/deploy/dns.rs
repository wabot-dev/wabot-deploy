//! Does this name reach this node?
//!
//! Asked before a hostname is accepted, not after. A route for a name
//! that points somewhere else is a certificate request that fails, a
//! page that never loads, and an operator with no way to tell which of
//! the two went wrong.
//!
//! ## Compared against the node's own domain, not its IP
//!
//! The node cannot reliably know its public address: behind NAT it
//! sees a private one, and asking an external service for it is a
//! dependency on somebody else's uptime. But the node's own domain
//! already resolves to wherever the world reaches it — that is what
//! makes the console reachable — so "does this name resolve to the
//! same place" is answerable with two lookups and no assumptions.
//!
//! ## The wildcard probe
//!
//! `*.example.com` cannot be queried directly; a wildcard record is
//! only visible by asking for a name that nothing else defines. So the
//! node asks for a random one. If `k7f3p9q2.example.com` resolves to
//! where `example.com` does, a wildcard is in place and every service
//! can have a subdomain without another DNS record.

use std::collections::BTreeSet;
use std::net::IpAddr;

/// What a lookup said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// It resolves here — the same addresses as the node's own domain.
    Here,
    /// It resolves, but somewhere else.
    Elsewhere {
        found: Vec<String>,
        node: Vec<String>,
    },
    /// Nothing answered for it.
    Missing,
    /// The node's own domain does not resolve either, so there is
    /// nothing to compare against. Distinct from `Missing`: this one is
    /// the node's problem, not the operator's hostname.
    NoReference,
}

impl Resolution {
    pub fn ok(&self) -> bool {
        matches!(self, Resolution::Here)
    }

    /// What to tell somebody staring at a form.
    pub fn explain(&self, hostname: &str) -> String {
        match self {
            Resolution::Here => format!("{hostname} resolves to this node"),
            Resolution::Elsewhere { found, node } => format!(
                "{hostname} resolves to {} — this node answers at {}. \
                 Point it here and try again.",
                found.join(", "),
                node.join(", ")
            ),
            Resolution::Missing => format!(
                "{hostname} does not resolve. Add the DNS record, wait for it to \
                 propagate, and try again."
            ),
            Resolution::NoReference => {
                "this node's own domain does not resolve, so there is nothing to check \
                 against — set node.domain to a name that points here"
                    .into()
            }
        }
    }
}

/// Does `hostname` resolve to the same place as `node_domain`?
pub async fn resolves_here(hostname: &str, node_domain: &str) -> Resolution {
    let node = lookup(node_domain).await;
    if node.is_empty() {
        return Resolution::NoReference;
    }

    let found = lookup(hostname).await;
    if found.is_empty() {
        return Resolution::Missing;
    }

    // Intersecting rather than comparing whole sets: a name behind a
    // pair of A records, or one that resolves to both a v4 and a v6
    // address, still reaches this node if any address matches. The
    // question is "does traffic arrive here", not "are these records
    // identical".
    if found.intersection(&node).next().is_some() {
        Resolution::Here
    } else {
        Resolution::Elsewhere {
            found: found.iter().map(ToString::to_string).collect(),
            node: node.iter().map(ToString::to_string).collect(),
        }
    }
}

/// The address this machine would use to reach the world.
///
/// **Not proof of anything, and offered as one number beside another.**
/// The module docs say why the checks above refuse to depend on this:
/// behind NAT a machine sees a private address while the world reaches
/// it at a different one, so a rule built on this would be wrong on
/// every node behind a router.
///
/// It earns its place in exactly one situation — a node that has just
/// been *restored onto different hardware*, where the comparison the
/// rest of this module makes is structurally blind. `resolves_here`
/// compares a name against the node's own domain, and on a rebuilt
/// machine that domain still resolves: to the machine that died. Every
/// name agrees with it, everything reads `Here`, and nothing arrives.
///
/// So `restore-node` prints this beside where the names point and says
/// it cannot decide. The operator knows whether there is a NAT in front
/// of this box; the node does not.
///
/// No packet is sent. Connecting a UDP socket only consults the routing
/// table, which is the question being asked: *which of my addresses
/// would leave this machine?*
pub fn outbound_address() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // A public address that need not answer, and will not be contacted.
    socket.connect("192.0.2.1:443").ok()?;
    socket.local_addr().ok().map(|address| address.ip())
}

/// Is there a wildcard record under `node_domain`?
///
/// Probed with a random label, because a wildcard is invisible to a
/// direct query: it answers for names nothing else defines, so the
/// only way to see it is to ask for one.
pub async fn wildcard_works(node_domain: &str) -> bool {
    let probe = format!("{}.{node_domain}", random_label());
    matches!(resolves_here(&probe, node_domain).await, Resolution::Here)
}

/// A label no zone would have defined on purpose.
///
/// Long enough that a collision with a real record is not a thing to
/// think about, and lowercase alphanumeric so it is a valid label
/// everywhere.
fn random_label() -> String {
    format!("wd-probe-{}", wabot::prelude::password::generate(10)).to_ascii_lowercase()
}

/// The addresses a hostname resolves to.
///
/// Through the operating system's resolver — `ToSocketAddrs` — rather
/// than a DNS client of our own. That means the node sees exactly what
/// its own outgoing connections will see, including `/etc/hosts` and
/// whatever search domains are configured, which is the answer that
/// actually matters. It also means no resolver library, no timeout
/// knobs, and no second opinion about what DNS is.
pub async fn lookup(hostname: &str) -> BTreeSet<IpAddr> {
    let hostname = hostname.trim().trim_end_matches('.').to_string();
    if hostname.is_empty() {
        return BTreeSet::new();
    }

    // Blocking, and on its own thread: the system resolver can sit on
    // a socket for seconds, and doing that on an executor thread stops
    // the console answering while somebody waits for a DNS check.
    tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;

        // The port is required by the API and irrelevant to the
        // answer; 443 is the one this node would use anyway.
        match format!("{hostname}:443").to_socket_addrs() {
            Ok(addresses) => addresses.map(|address| address.ip()).collect(),
            Err(_) => BTreeSet::new(),
        }
    })
    .await
    .unwrap_or_default()
}

/// The subdomain to propose for a service.
pub fn suggested_hostname(service_slug: &str, project_slug: &str, node_domain: &str) -> String {
    format!("{service_slug}.{project_slug}.{node_domain}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The number `restore-node` shows beside a DNS answer has to be an
    /// address this machine actually holds.
    ///
    /// The way this trick fails is quiet and specific: a UDP socket
    /// that is bound and never connected reports `0.0.0.0`, which is a
    /// perfectly good-looking answer and means nothing. Printed beside
    /// a real address, it would invite somebody rebuilding a node to
    /// conclude their DNS is wrong when it is not.
    ///
    /// `None` is a legitimate answer — a machine with no route out —
    /// so the claim is about what it says when it says anything.
    #[test]
    fn the_address_this_machine_goes_out_from_is_a_real_one() {
        let Some(address) = outbound_address() else {
            return;
        };
        assert!(
            !address.is_unspecified(),
            "{address} is the wildcard, not an address this machine holds"
        );
        assert!(
            !address.is_loopback(),
            "{address} is loopback; the route asked for was a public address"
        );
    }

    #[test]
    fn the_suggestion_reads_as_a_hostname() {
        assert_eq!(
            suggested_hostname("web", "my-api", "node.example"),
            "web.my-api.node.example"
        );
    }

    /// The probe has to be a name nobody has defined, or it answers
    /// "wildcard" for a zone that merely has that one record.
    #[test]
    fn the_probe_label_is_random_and_valid() {
        let first = random_label();
        let second = random_label();

        assert_ne!(first, second);
        assert!(first.starts_with("wd-probe-"), "{first}");
        assert!(first.len() <= 63, "a DNS label is at most 63 bytes");
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{first} is not a valid label"
        );
    }

    /// Every branch has to say what to do next. A check that reports
    /// "failed" and nothing else is one the operator cannot act on.
    #[test]
    fn every_outcome_explains_itself() {
        let cases = [
            Resolution::Here,
            Resolution::Elsewhere {
                found: vec!["203.0.113.9".into()],
                node: vec!["198.51.100.4".into()],
            },
            Resolution::Missing,
            Resolution::NoReference,
        ];
        for case in cases {
            let message = case.explain("api.example.com");
            assert!(!message.is_empty());
            if !case.ok() {
                assert!(
                    message.contains("try again") || message.contains("node.domain"),
                    "{message} does not say what to do"
                );
            }
        }
    }

    #[test]
    fn a_wrong_answer_names_both_sides() {
        let message = Resolution::Elsewhere {
            found: vec!["203.0.113.9".into()],
            node: vec!["198.51.100.4".into()],
        }
        .explain("api.example.com");

        assert!(message.contains("203.0.113.9"), "where it points");
        assert!(message.contains("198.51.100.4"), "where the node is");
    }

    /// localhost is the one name that resolves the same everywhere, so
    /// it is the only real lookup a test can make without a network.
    #[tokio::test]
    async fn a_name_that_resolves_where_the_node_does_is_here() {
        assert_eq!(
            resolves_here("localhost", "localhost").await,
            Resolution::Here
        );
    }

    #[tokio::test]
    async fn a_name_that_resolves_to_nothing_is_missing() {
        // `.invalid` is reserved by RFC 2606 precisely so it can never
        // resolve — no test should depend on somebody not registering
        // a domain.
        let outcome = resolves_here("nothing.invalid", "localhost").await;
        assert_eq!(outcome, Resolution::Missing, "{outcome:?}");
    }

    #[tokio::test]
    async fn a_node_whose_own_domain_is_unresolvable_says_so() {
        let outcome = resolves_here("localhost", "also-nothing.invalid").await;
        assert_eq!(outcome, Resolution::NoReference);
    }
}
