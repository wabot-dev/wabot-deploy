use hypertext::prelude::*;

use crate::domain::values::capability::Capability;
use crate::features::network::texts::t;
use crate::infra::network::Node;
use crate::shell::components::node_capabilities;
use crate::shell::components::node_header::{node_header, NodePlace};
use crate::shell::layouts::layout;

/// What this node decides about another one: what it will do for it,
/// whether it is in service, and whether it is listed at all.
///
/// That node's own settings are on its own console. A grant belongs to
/// the machine that made it.
pub struct RemoteNodeSettings<'a> {
    pub node: &'a Node,
    /// Every capability, so the form cannot be missing one.
    pub granted: &'a [(Capability, bool)],
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a RemoteNodeSettings<'a>) -> impl Renderable + 'a {
    let node = page.node;
    rsx! {
        (layout::style_tag())
        (node_header(node, NodePlace::Settings))
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }
        (node_capabilities::grants_card(node, page.granted))

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
