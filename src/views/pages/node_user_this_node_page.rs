use hypertext::prelude::*;

use crate::deploy::Committed;
use crate::network::Node;
use crate::node::disk::Breakdown;
use crate::node::memory::Snapshot;
use crate::views::components::node_usage;
use crate::views::layouts::layout;

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
}

pub fn render<'a>(page: &'a ThisNode<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <div class="split">
            <div class="stack-sm">
                <h1>(&page.node.name)</h1>
                <p class="slug-preview">("wabot-deploy ")(crate::api::VERSION)</p>
            </div>
        </div>

        // One island around both cards: the stream replaces figures in
        // each, and the runtime tears it down when this host leaves the
        // DOM. It used to be an inline `<script>`, which a boosted
        // navigation never runs — arriving here by clicking a link left
        // the page a snapshot with no sign that it was one.
        (node_usage::live_cards(&page.node.id, page.disk, page.committed, page.snapshot, page.reserves))

        // Outside the island: none of it streams, and a card inside the
        // host would be replaced by an update that has nothing to say
        // about it.
        (node_usage::network_card(page.node))
    }
}
