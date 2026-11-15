use hypertext::prelude::*;

use crate::features::network::node_connections_component::{
    connection_card, connections_card, Links,
};
use crate::features::network::node_usage_component;
use crate::infra::network::Node;
use crate::shell::components::node_header::{node_header, NodePlace};
use crate::shell::layouts::layout;

/// A node's place on the network.
///
/// For this machine that is every other node and the tunnel to each; for
/// another one it is the single tunnel between the two, because this node
/// knows nothing of the ones that node keeps with anybody else.
pub struct NodeConnections<'a> {
    pub node: &'a Node,
    pub links: Links<'a>,
}

pub fn render<'a>(page: &'a NodeConnections<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        (node_header(page.node, NodePlace::Connections))
        @if page.node.is_self {
            (connections_card(&page.links))
        } @else {
            (connection_card(page.node, &page.links))
        }
        (node_usage_component::network_card(page.node))
    }
}
