use hypertext::prelude::*;

use crate::features::network::node_connections_component::{connections_card, Links};
use crate::features::node::attention_card_component;
use crate::features::node::concern_response::ConcernResponse;
use crate::features::node::texts::t;
use crate::infra::network::Node;
use crate::infra::node::disk::Filesystem;
use crate::infra::node::memory::{self, Snapshot};
use crate::shell::layouts::layout;

/// Where whoever runs the machine lands: is it all right, is it reaching
/// the others, and does anything need them.
///
/// Nothing here is measured for this page. It puts three things the node
/// already knows where a person arrives, which used to mean opening the
/// network area, this node's settings and the list of projects to
/// assemble one answer.
pub struct Status<'a> {
    pub node: Option<&'a Node>,
    pub concerns: &'a [ConcernResponse],
    pub memory: &'a Snapshot,
    pub disk: Filesystem,
    pub domain: Option<&'a str>,
    pub links: Links<'a>,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a Status<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <h1>(t("Status"))</h1>
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }

        // Said when there is nothing, here and only here. Above a list of
        // projects an empty card was wallpaper; on the page whose whole
        // question is "is anything wrong", silence is not an answer.
        @if page.concerns.is_empty() {
            <div class="stack">
                <p class="card-label">(t("Needs you"))</p>
                <div class="card">
                    <p class="tile-detail">(t("Nothing does."))</p>
                </div>
            </div>
        } @else {
            (attention_card_component::card(page.concerns))
        }

        @if let Some(node) = page.node {
            <div class="stack">
                <p class="card-label">(t("This node"))</p>
                <div class="card stack">
                    <div class="split">
                        <a class="tile-name" href=(format!("/nodes/{}", node.id))>(&node.name)</a>
                        <span class="badge badge-success">
                            <span class="dot dot-success"></span>(t("Serving"))
                        </span>
                    </div>
                    <dl class="kv">
                        <dt>(t("Version"))</dt>
                        <dd>("wabot-deploy ")(crate::infra::update::VERSION)</dd>
                        <dt>(t("Memory"))</dt>
                        <dd>
                            (memory::human(page.memory.used()))(t(" of "))
                            (memory::human(page.memory.total))
                        </dd>
                        <dt>(t("Disk"))</dt>
                        <dd>
                            (memory::human(page.disk.used()))(t(" of "))
                            (memory::human(page.disk.total))
                            (" · ")(memory::human(page.disk.available))(t(" free"))
                        </dd>
                        <dt>(t("Domain"))</dt>
                        <dd>(page.domain.unwrap_or("—"))</dd>
                    </dl>
                </div>
            </div>
        }

        (connections_card(&page.links))
    }
}
