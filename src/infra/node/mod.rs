//! What this machine is, and what it is spending.
//!
//! ## The list moved
//!
//! This module used to answer "what nodes are there" with a synthetic
//! list of one, so that the page and the data would already be the
//! shape a second node arrived into — a list that starts as a detail
//! page never becomes a list without breaking every link into it. That
//! second node is now possible, so the list is a table: see
//! [`crate::infra::network`]. What is left here is what only *this* machine
//! can answer.
//!
//! ## Memory, attributed rather than totalled
//!
//! "Used" is a number every tool shows and nobody can act on. What an
//! operator of *this* node needs is which part of it is the platform
//! and which is what they deployed — because the platform's share is
//! the number this whole product is trying to keep small.
//!
//! So the reading is broken up: this process, containerd, one shim per
//! running container, the containers themselves, and everything else.
//! The shims are their own line rather than folded into either side —
//! they exist *because* there is a container, but they are the
//! runtime's overhead, not the application's, and hiding them in
//! either column would answer a question nobody asked.

pub mod cpu;
pub mod disk;
pub mod memory;
pub mod settings;

/// What to call this machine in a list.
///
/// The domain if it answers to one, the kernel's hostname if not, and
/// a placeholder if even that is missing — a blank row in a list of
/// nodes is worse than a vague one.
pub fn name(domain: Option<&str>) -> String {
    let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());

    domain
        .map(str::to_string)
        .or(hostname)
        .unwrap_or_else(|| "this node".into())
}

/// Where this node answers, as `host` or `host:port`, from wherever the
/// person reading is: the domain, else the address this machine reaches
/// the world from, and `localhost` only for a machine with no route at all.
/// See `setup_token::url` for why it is not `localhost` first.
///
/// One answer for every sentence that tells somebody where to point
/// something. The push token's `docker login` said `this node` instead,
/// which is a command no terminal can run — found on a VM node.
pub fn authority(domain: Option<&str>, https_port: u16) -> String {
    let host = match domain {
        Some(domain) => domain.to_string(),
        None => match crate::infra::deploy::dns::outbound_address() {
            Some(std::net::IpAddr::V6(address)) => format!("[{address}]"),
            Some(address) => address.to_string(),
            None => "localhost".into(),
        },
    };
    match https_port {
        443 => host,
        port => format!("{host}:{port}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_is_reached_at_its_domain_with_the_port_only_when_it_is_not_443() {
        assert_eq!(authority(Some("node.example"), 443), "node.example");
        assert_eq!(authority(Some("node.example"), 8443), "node.example:8443");
    }

    /// A node with no domain is still somewhere a terminal can name.
    #[test]
    fn a_node_with_no_domain_is_reached_at_an_address() {
        let reached = authority(None, 443);
        assert!(!reached.contains(' '), "{reached}");
        assert!(!reached.is_empty());
    }

    #[test]
    fn a_node_with_a_domain_is_named_by_it() {
        assert_eq!(name(Some("node.example")), "node.example");
    }

    /// A node with no domain still has to be called something, or the
    /// list shows a blank row.
    #[test]
    fn a_node_without_a_domain_still_has_a_name() {
        assert!(!name(None).is_empty());
    }
}
