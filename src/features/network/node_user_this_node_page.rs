use hypertext::prelude::*;

use crate::features::network::node_usage_component;
use crate::infra::deploy::Committed;
use crate::infra::network::Node;
use crate::infra::node::disk::Breakdown;
use crate::infra::node::memory::Snapshot;
use crate::web::views::layouts::layout;

/// This machine's own row on the network page.
///
/// Two cards and no forms: what is *editable* about this node lives at
/// `/node`, and mixing the two was the page that had to be split.
pub struct ThisNode<'a> {
    pub node: &'a Node,
    pub disk: &'a Breakdown,
    pub committed: &'a Committed,
    pub snapshot: &'a Snapshot,
    pub reserves: (Option<u64>, Option<u32>),
    /// **A refusal that reaches this page has to be readable on it.**
    /// It had no error at all, so anything redirected here with one
    /// showed a page that had simply moved and said nothing — which is
    /// how a correct refusal about a name that does not resolve reached
    /// nobody. Nothing points here with an error any more; this is so
    /// that the next thing which does cannot be silent.
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ThisNode<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }
        <div class="split">
            <div class="stack-sm">
                <h1>(&page.node.name)</h1>
                <p class="slug-preview">("wabot-deploy ")(crate::infra::update::VERSION)</p>
            </div>
        </div>

        // One island around both cards: the stream replaces figures in
        // each, and the runtime tears it down when this host leaves the
        // DOM. It used to be an inline `<script>`, which a boosted
        // navigation never runs — arriving here by clicking a link left
        // the page a snapshot with no sign that it was one.
        (node_usage_component::live_cards(&page.node.id, page.disk, page.committed, page.snapshot, page.reserves))

        // Outside the island: none of it streams, and a card inside the
        // host would be replaced by an update that has nothing to say
        // about it.
        (node_usage_component::network_card(page.node))
    }
}
