//! What this machine is, and what it is spending.
//!
//! ## One node, in the plural
//!
//! There is exactly one node today — this one — and the console lists
//! it as a list of one. That is not decoration: the shape of the page
//! and the shape of the data are what a second node would arrive into,
//! and a list that starts as a detail page never becomes a list
//! without breaking every link into it.
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

pub mod memory;

use crate::config::Config;

/// A node, as the console lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Stable within this installation. `local` while a node can only
    /// describe itself; a joined node would carry its own.
    pub id: String,
    /// What it answers to, or its hostname when it has no domain.
    pub name: String,
    pub domain: Option<String>,
    pub version: &'static str,
    /// True for the node serving this console.
    pub is_self: bool,
}

pub const LOCAL_ID: &str = "local";

impl Node {
    pub fn local(config: &Config) -> Self {
        let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty());

        Self {
            id: LOCAL_ID.into(),
            name: config
                .node
                .domain
                .clone()
                .or(hostname)
                .unwrap_or_else(|| "this node".into()),
            domain: config.node.domain.clone(),
            version: crate::api::VERSION,
            is_self: true,
        }
    }
}

/// Every node this console knows about.
///
/// One, and the plural is the point — see the module docs.
pub fn all(config: &Config) -> Vec<Node> {
    vec![Node::local(config)]
}

pub fn find(config: &Config, id: &str) -> Option<Node> {
    all(config).into_iter().find(|node| node.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_with_a_domain_is_named_by_it() {
        let mut config = Config::default();
        config.node.domain = Some("node.example".into());

        let node = Node::local(&config);
        assert_eq!(node.name, "node.example");
        assert_eq!(node.id, LOCAL_ID);
        assert!(node.is_self);
    }

    /// A node with no domain still has to be called something, or the
    /// list shows a blank row.
    #[test]
    fn a_node_without_a_domain_still_has_a_name() {
        let node = Node::local(&Config::default());
        assert!(!node.name.is_empty());
    }

    #[test]
    fn the_list_contains_this_node() {
        let config = Config::default();
        let nodes = all(&config);

        assert_eq!(nodes.len(), 1);
        assert_eq!(find(&config, LOCAL_ID), Some(nodes[0].clone()));
        assert_eq!(find(&config, "somewhere-else"), None);
    }
}
