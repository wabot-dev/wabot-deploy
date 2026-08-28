use hypertext::prelude::*;

use crate::network::capability::Capability;
use crate::network::errand::Record;
use crate::network::Node;
use crate::views::components::{node_capabilities, node_errands, node_settings_cards, node_usage};
use crate::views::layouts::layout;
use crate::views::texts::language::t;

/// A node that is **not** this one.
///
/// It has no memory reading and no certificate: both are answers only
/// the machine itself can give, and nothing asks it anything yet. So the
/// page says what is known rather than rendering cards full of
/// "unknown" — which is why this is its own page and not a branch inside
/// [`super::node_user_this_node_page`].
pub struct RemoteNode<'a> {
    pub node: &'a Node,
    pub orders: &'a [Record],
    /// Every capability, so the form cannot be missing one.
    pub granted: &'a [(Capability, bool)],
    pub releases: &'a [String],
    /// The release list could not be read. A card with an empty list and
    /// a sentence saying so, never a page that does not render: this
    /// console is opened when a node is unwell, and GitHub being
    /// unreachable is one of the ways that happens.
    pub catalogue_failed: bool,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a RemoteNode<'a>) -> impl Renderable + 'a {
    let node = page.node;
    rsx! {
        (layout::style_tag())
        <div class="split">
            <div class="stack-sm">
                <h1>(&node.name)</h1>
                <p class="slug-preview">(t("joined · "))(node_usage::reach(node))</p>
            </div>
        </div>
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }
        (node_usage::network_card(node))

        // What was asked of this node and how it went. It stays: the
        // form above it — "run a service there" — is what went, and this
        // is the record of every instruction, including the ones the
        // service page now sends.
        (node_usage::usage_card(node, node.last_seen_at.map(layout::when)))
        (node_settings_cards::manage_card(node, page.releases, page.catalogue_failed))
        (node_capabilities::grants_card(node, page.granted))
        (node_errands::errands_card(&node.id, page.orders))

        <section class="card stack">
            <p class="card-label">(t("Take it out of service"))</p>
            @if let Some(since) = node.draining_at {
                <p class="field-hint">(t("Draining since "))(layout::when(since))(t(". \
                     New copies go elsewhere. The ones already moved stop on \
                     that machine when it next collects its instructions — \
                     which for a node that is down is whenever it comes \
                     back."))</p>
                <form method="post" action=(format!("/network/{}/drain", node.id))>
                    <input type="hidden" name="draining" value="off">
                    <button class="btn btn-ghost" type="submit">
                        (t("Put it back in service"))
                    </button>
                </form>
            } @else {
                <p class="field-hint">(t("Moves the copies this node put there to \
                     somewhere with room, and stops choosing it for new ones. \
                     Copies that arrived from another node are not this one's \
                     to move; a database copy that accepts writes is named \
                     rather than moved, because promoting is a decision \
                     somebody makes."))</p>
                <form method="post" action=(format!("/network/{}/drain", node.id))>
                    <input type="hidden" name="draining" value="on">
                    <button class="btn btn-ghost" type="submit">
                        (t("Drain"))
                    </button>
                </form>
            }
        </section>

        <section class="card stack">
            <p class="card-label">(t("Forget this node"))</p>
            <p class="field-hint">(t("This node stops listing it. It is one direction only: the \
                 other node still holds this one as an authority until \
                 somebody revokes it there, from its own console. A grant \
                 belongs to the node that made it."))</p>
            <form method="post" action=(format!("/network/forget/{}", node.id))>
                <button class="btn btn-ghost destructive" type="submit">
                    (t("Forget"))
                </button>
            </form>
        </section>
    }
}
