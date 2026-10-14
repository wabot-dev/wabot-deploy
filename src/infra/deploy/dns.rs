//! Does this name reach this node?
//!
//! Asked before a hostname is accepted, not after. A route for a name
//! that points somewhere else is a certificate request that fails, a
//! page that never loads, and an operator with no way to tell which of
//! the two went wrong.
//!
//! ## Compared against a node's own domain, not its IP
//!
//! A node cannot reliably know its public address: behind NAT it sees a
//! private one, and asking an external service for it is a dependency on
//! somebody else's uptime. But a node's own domain already resolves to
//! wherever the world reaches it — that is what makes its console
//! reachable — so "does this name resolve to the same place" is
//! answerable with two lookups and no assumptions.
//!
//! ## And not only against *this* node
//!
//! **This was the bug.** The question was "does it point here", and a
//! service can perfectly well be served by another node on the network —
//! or by several at once. That is what `service_edge` has meant since
//! phase 7, and the form that accepts a hostname was refusing every name
//! pointing at any machine but this one.
//!
//! So the comparison is against **every node that could answer for it**:
//! public, dialable, and — for a node that is not this one — one that
//! `allows` this node to ask (see `network::edge_candidates`). Each is
//! resolved by its own advertised name, because that is what a node
//! publishes about itself; an endpoint is `{domain}:{port}`, never an
//! address.
//!
//! What comes back is **which** of them the name reaches, so the caller
//! can write the rows that make it true rather than assuming this node.
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

/// A node that could answer for a name, and the name that says where it
/// is.
///
/// `at` is the node's *own* name — its domain, or the host part of the
/// endpoint it advertises. Resolving that is how this module learns an
/// address without asking any node to know its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub node_id: String,
    /// For the sentence a person reads. Never a decision.
    pub node_name: String,
    pub at: String,
    pub is_self: bool,
}

/// One node a name was found to reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reached {
    pub node_id: String,
    pub node_name: String,
    pub is_self: bool,
}

/// What a lookup said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// It reaches these nodes — every one whose own name resolves to an
    /// address this one does too. **The list, not a yes**: it is what the
    /// caller writes into `service_edge`, because the nodes the name
    /// actually points at are the ones that can answer for it.
    Reaches(Vec<Reached>),
    /// It resolves, and to nothing that could answer for it.
    Elsewhere {
        found: Vec<String>,
        /// Where each node that could have answered actually is, as
        /// `name → addresses`, so the operator can see what to point at.
        known: Vec<String>,
    },
    /// Nothing answered for it.
    Missing,
    /// No node on this network has a name that resolves, so there is
    /// nothing to compare against. Distinct from `Missing`: this one is
    /// the network's problem, not the operator's hostname.
    NoReference,
}

impl Resolution {
    pub fn ok(&self) -> bool {
        matches!(self, Resolution::Reaches(reached) if !reached.is_empty())
    }

    /// Whether it reaches **this** node, which is a different question
    /// from `ok`: a certificate from a public authority is validated by
    /// a challenge served from here, so a name that reaches only some
    /// other machine is one this node must not order for.
    pub fn reaches_here(&self) -> bool {
        matches!(self, Resolution::Reaches(reached) if reached.iter().any(|one| one.is_self))
    }

    /// The nodes it reaches, for the caller that has rows to write.
    pub fn nodes(&self) -> Vec<String> {
        match self {
            Resolution::Reaches(reached) => reached.iter().map(|one| one.node_id.clone()).collect(),
            _ => Vec::new(),
        }
    }

    /// What to tell somebody staring at a form.
    pub fn explain(&self, hostname: &str) -> String {
        match self {
            Resolution::Reaches(reached) if reached.iter().any(|one| one.is_self) => {
                match reached.len() {
                    1 => format!("{hostname} resolves to this node"),
                    _ => format!(
                        "{hostname} resolves to this node and to {}",
                        reached
                            .iter()
                            .filter(|one| !one.is_self)
                            .map(|one| one.node_name.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                }
            }
            Resolution::Reaches(reached) => format!(
                "{hostname} resolves to {}, which can answer for it",
                reached
                    .iter()
                    .map(|one| one.node_name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Resolution::Elsewhere { found, known } => format!(
                "{hostname} resolves to {} — no node that can serve it answers there. \
                 The ones that could: {}. Point the name at one of them and try again.",
                found.join(", "),
                known.join("; ")
            ),
            Resolution::Missing => format!(
                "{hostname} does not resolve. Add the DNS record, wait for it to \
                 propagate, and try again."
            ),
            Resolution::NoReference => {
                "no node on this network has a name that resolves, so there is nothing to \
                 check against — give this node a domain, or one that can serve it"
                    .into()
            }
        }
    }
}

/// Does `hostname` resolve to the same place as `node_domain`?
///
/// The one-node question, kept because two callers ask exactly it: the
/// form that sets **this** node's own domain, and `acme::ensure` before
/// it spends an order — a challenge is served from here, so "here" is
/// the only answer that helps.
pub async fn resolves_here(hostname: &str, node_domain: &str) -> Resolution {
    resolves_to(
        hostname,
        &[Reference {
            node_id: String::new(),
            node_name: "this node".into(),
            at: node_domain.to_string(),
            is_self: true,
        }],
    )
    .await
}

/// Which of these nodes does `hostname` reach?
///
/// Every reference is resolved, then intersected with what the hostname
/// resolves to. **Intersecting rather than comparing whole sets**: a name
/// behind a pair of A records, or one that resolves to both a v4 and a v6
/// address, still reaches a node if any address matches. The question is
/// "does traffic arrive there", not "are these records identical" — and
/// with several nodes behind one name, arriving at more than one is the
/// point rather than a problem.
pub async fn resolves_to(hostname: &str, references: &[Reference]) -> Resolution {
    let mut known = Vec::new();
    let mut addresses = Vec::new();
    for reference in references {
        let at = lookup(&reference.at).await;
        if at.is_empty() {
            continue;
        }
        known.push(format!(
            "{} at {}",
            reference.node_name,
            at.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
        addresses.push((reference, at));
    }
    if addresses.is_empty() {
        return Resolution::NoReference;
    }

    let found = lookup(hostname).await;
    if found.is_empty() {
        return Resolution::Missing;
    }

    let reached: Vec<Reached> = addresses
        .iter()
        .filter(|(_, at)| found.intersection(at).next().is_some())
        .map(|(reference, _)| Reached {
            node_id: reference.node_id.clone(),
            node_name: reference.node_name.clone(),
            is_self: reference.is_self,
        })
        .collect();

    match reached.is_empty() {
        false => Resolution::Reaches(reached),
        true => Resolution::Elsewhere {
            found: found.iter().map(ToString::to_string).collect(),
            known,
        },
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
    resolves_here(&probe, node_domain).await.ok()
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

/// Every node that could answer for a hostname, and the name that
/// says where each one is.
///
/// A node's endpoint is `{domain}:{port}` — a **name**, decided that
/// way so a node behind NAT still advertises something true — so the
/// host part of it is what a lookup can compare. This node uses its
/// own domain, which is the same string by a different route.
pub async fn references(
    database: &wabot::sqlite::SqliteDatabase,
) -> Result<Vec<Reference>, String> {
    let mut references = Vec::new();
    for node in crate::infra::network::edge_candidates(database)
        .await
        .map_err(|error| error.to_string())?
    {
        let at = match node.is_self {
            true => crate::infra::node::settings::domain(database).await,
            // The port is not part of a lookup, and an endpoint
            // without one would be a name that never resolves.
            false => node
                .endpoint
                .as_deref()
                .map(|endpoint| endpoint.rsplit_once(':').map_or(endpoint, |(host, _)| host))
                .map(str::to_string),
        };
        let Some(at) = at.filter(|at| !at.trim().is_empty()) else {
            continue;
        };
        references.push(Reference {
            node_id: node.id.clone(),
            node_name: node.name.clone(),
            at,
            is_self: node.is_self,
        });
    }
    Ok(references)
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
            Resolution::Reaches(vec![Reached {
                node_id: "nd-self".into(),
                node_name: "this node".into(),
                is_self: true,
            }]),
            // Reached, and not by this one: the sentence has to name the
            // machine that will answer, or the operator is left wondering
            // whether it worked.
            Resolution::Reaches(vec![Reached {
                node_id: "nd-other".into(),
                node_name: "alpine".into(),
                is_self: false,
            }]),
            Resolution::Elsewhere {
                found: vec!["203.0.113.9".into()],
                known: vec!["this node at 198.51.100.4".into()],
            },
            Resolution::Missing,
            Resolution::NoReference,
        ];
        for case in cases {
            let message = case.explain("api.example.com");
            assert!(!message.is_empty());
            if !case.ok() {
                assert!(
                    message.contains("try again") || message.contains("domain"),
                    "{message} does not say what to do"
                );
            }
        }
    }

    #[test]
    fn a_wrong_answer_names_both_sides() {
        let message = Resolution::Elsewhere {
            found: vec!["203.0.113.9".into()],
            known: vec![
                "this node at 198.51.100.4".into(),
                "alpine at 192.0.2.7".into(),
            ],
        }
        .explain("api.example.com");

        assert!(message.contains("203.0.113.9"), "where it points");
        assert!(message.contains("198.51.100.4"), "where this node is");
        // And every other node that could have served it, because
        // pointing the name at one of them is the way out and the
        // operator cannot pick from a list they were not shown.
        assert!(message.contains("alpine at 192.0.2.7"), "{message}");
    }

    /// A name on several nodes reaches several nodes.
    ///
    /// The bug this replaced: the answer was a yes or a no about *this*
    /// machine, so a service exposed through another node — which is what
    /// `service_edge` has meant since phase 7 — was refused at the form.
    /// What comes back now is which nodes, because those are the rows
    /// that make the name actually answerable.
    #[tokio::test]
    async fn a_name_can_reach_more_than_one_node() {
        // `localhost` is the one name that resolves the same everywhere,
        // so two references to it stand in for two nodes that share an
        // address — which is also a real case: two machines behind one
        // NAT.
        let references = [
            Reference {
                node_id: "nd-self".into(),
                node_name: "this node".into(),
                at: "localhost".into(),
                is_self: true,
            },
            Reference {
                node_id: "nd-other".into(),
                node_name: "alpine".into(),
                at: "localhost".into(),
                is_self: false,
            },
            // And one that resolves to nothing is simply not a reference:
            // it must not turn the answer into `NoReference`, which is
            // about having nothing to compare at all.
            Reference {
                node_id: "nd-gone".into(),
                node_name: "gone".into(),
                at: "nothing.invalid".into(),
                is_self: false,
            },
        ];

        let outcome = resolves_to("localhost", &references).await;
        assert_eq!(outcome.nodes(), vec!["nd-self", "nd-other"]);
        assert!(outcome.ok() && outcome.reaches_here());

        // A name that reaches only the other node is usable — and is not
        // a name this node may order a certificate for.
        let elsewhere = resolves_to("localhost", &references[1..]).await;
        assert_eq!(elsewhere.nodes(), vec!["nd-other"]);
        assert!(elsewhere.ok(), "it can be served");
        assert!(
            !elsewhere.reaches_here(),
            "and not by a challenge served from here"
        );
    }

    /// localhost is the one name that resolves the same everywhere, so
    /// it is the only real lookup a test can make without a network.
    #[tokio::test]
    async fn a_name_that_resolves_where_the_node_does_is_here() {
        let outcome = resolves_here("localhost", "localhost").await;
        assert!(outcome.ok() && outcome.reaches_here(), "{outcome:?}");
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
