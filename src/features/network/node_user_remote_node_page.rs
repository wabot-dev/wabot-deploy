use hypertext::prelude::*;

use crate::features::network::{node_errands_component, node_usage_component};
use crate::infra::network::errand::Record;
use crate::infra::network::Node;
use crate::shell::components::node_header::{node_header, NodePlace};
use crate::shell::layouts::layout;

/// A node that is **not** this one: how full it is, and what this node
/// has asked of it.
///
/// It has no memory reading and no certificate: both are answers only
/// the machine itself can give. So the page says what is known rather
/// than rendering cards full of "unknown" — which is why this is its own
/// page and not a branch inside [`super::node_user_this_node_page`].
pub struct RemoteNode<'a> {
    pub node: &'a Node,
    pub orders: &'a [Record],
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a RemoteNode<'a>) -> impl Renderable + 'a {
    let node = page.node;
    rsx! {
        (layout::style_tag())
        (node_header(node, NodePlace::Overview))
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }
        (node_usage_component::usage_card(node, node.last_seen_at.map(layout::when)))
        // The record of every instruction, including the ones a
        // service's page sends.
        (node_errands_component::errands_card(&node.id, page.orders))
    }
}
