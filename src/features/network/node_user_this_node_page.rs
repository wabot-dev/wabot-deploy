use hypertext::prelude::*;

use crate::features::network::node_usage_component;
use crate::infra::deploy::Committed;
use crate::infra::network::Node;
use crate::infra::node::disk::Breakdown;
use crate::infra::node::memory::Snapshot;
use crate::shell::components::node_header::{node_header, NodePlace};
use crate::shell::layouts::layout;

/// This machine, as one of the nodes: what it is using.
///
/// No forms. What is *editable* about it is its Settings tab, and mixing
/// the two was the page that had to be split.
pub struct ThisNode<'a> {
    pub node: &'a Node,
    pub disk: &'a Breakdown,
    pub committed: &'a Committed,
    pub snapshot: &'a Snapshot,
    pub reserves: (Option<u64>, Option<u32>),
    pub guard: Option<&'a crate::infra::runtime::guard::Reading>,
    /// **A refusal that reaches this page has to be readable on it.**
    /// It had no error at all, so anything redirected here with one
    /// showed a page that had simply moved and said nothing.
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ThisNode<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        (node_header(page.node, NodePlace::Overview))
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }

        // One island around both cards: the stream replaces figures in
        // each, and the runtime tears it down when this host leaves the
        // DOM.
        (node_usage_component::live_cards(&page.node.id, page.disk, page.committed, page.snapshot, page.reserves, page.guard))
    }
}
