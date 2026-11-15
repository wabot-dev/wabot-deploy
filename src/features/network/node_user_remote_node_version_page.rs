use hypertext::prelude::*;

use crate::features::network::node_errands_component;
use crate::infra::network::errand::Record;
use crate::infra::network::Node;
use crate::shell::components::node_header::{node_header, NodePlace};
use crate::shell::components::node_settings_cards;
use crate::shell::layouts::layout;

/// The release another node runs, and the one it has been asked for.
pub struct RemoteNodeVersion<'a> {
    pub node: &'a Node,
    pub releases: &'a [String],
    /// The release list could not be read. A card with an empty list and
    /// a sentence saying so, never a page that does not render: this
    /// console is opened when a node is unwell, and GitHub being
    /// unreachable is one of the ways that happens.
    pub catalogue_failed: bool,
    pub orders: &'a [Record],
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a RemoteNodeVersion<'a>) -> impl Renderable + 'a {
    let node = page.node;
    rsx! {
        (layout::style_tag())
        (node_header(node, NodePlace::Version))
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }
        (node_settings_cards::manage_card(node, page.releases, page.catalogue_failed))
        // Under the form that sends them: a release that node refuses
        // says why on its instruction, and the form's own hint points
        // here.
        (node_errands_component::errands_card(&node.id, page.orders))
    }
}
