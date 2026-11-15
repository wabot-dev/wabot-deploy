use hypertext::prelude::*;

use crate::domain::values::capability::Capability;
use crate::features::network::node_connections_component::{tunnel_badge, Links};
use crate::features::network::texts::t;
use crate::features::network::{node_enrolment_component, node_usage_component};
use crate::infra::network::token::JoinToken;
use crate::infra::network::{Authority, Node};
use crate::shell::layouts::layout;

pub struct Nodes<'a> {
    pub nodes: &'a [Node],
    pub links: Links<'a>,
    pub authorities: &'a [Authority],
    /// What each authority was actually agreed to. A row saying only
    /// "takes instructions from" would hide the difference between a
    /// node that may do everything here and one that may do nothing.
    pub agreed: &'a [(String, Vec<Capability>)],
    pub reviewing: Option<&'a (String, JoinToken)>,
    pub error: Option<&'a str>,
}

impl Nodes<'_> {
    fn held(&self, node_id: &str) -> &[Capability] {
        self.agreed
            .iter()
            .find(|(id, _)| id == node_id)
            .map(|(_, held)| held.as_slice())
            .unwrap_or_default()
    }
}

pub fn render<'a>(page: &'a Nodes<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <div class="split">
            <h1>(t("Nodes"))</h1>
            // The two things that change what is on this list, beside
            // it. They were links in the navigation, which is a list of
            // places — and neither is somewhere anybody stays.
            <div class="row">
                <a class="btn btn-secondary" href="/nodes/join">(t("Join a network"))</a>
                <a class="btn" href="/nodes/invite">(t("Invite a node"))</a>
            </div>
        </div>
        <p class="tagline">(t("This node, and the ones that have agreed to take \
             instructions from it."))</p>
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }

        @if let Some((token, terms)) = page.reviewing {
            (node_enrolment_component::review_card(token, terms))
        }

        <div class="grid">
            @for node in page.nodes {
                <a class="card tile" href=(format!("/nodes/{}", node.id))>
                    <div class="split">
                        <p class="tile-name">(&node.name)</p>
                        @if node.is_self {
                            <span class="badge badge-success">
                                <span class="dot dot-success"></span>(t("Serving"))
                            </span>
                        } @else {
                            // What the kernel says about the tunnel to
                            // it, and nothing warmer: a green dot for a
                            // node nobody has heard from would be this
                            // page inventing a health check.
                            (tunnel_badge(page.links.path_to(node)))
                        }
                    </div>
                    <p class="tile-detail">
                        @if node.is_self {
                            ("wabot-deploy ")(crate::infra::update::VERSION)(t(" · this node"))
                            @if let Some(address) = &node.overlay_ip {
                                (" · ")(address)
                            }
                        } @else {
                            (node_usage_component::reach(node))
                        }
                    </p>
                </a>
            }
        </div>

        @if !page.authorities.is_empty() {
            <section class="stack">
                <p class="card-label">(t("Takes instructions from"))</p>
                <p class="field-hint">(t("Nodes this one has granted authority to. Revoking is one \
                     row, and it takes effect here — a node that has been \
                     revoked can ask for nothing."))</p>
                <table>
                    <thead>
                        <tr>
                            <th>(t("Node"))</th><th>(t("May"))</th>
                            <th>(t("State"))</th><th></th>
                        </tr>
                    </thead>
                    <tbody>
                        @for authority in page.authorities {
                            (node_enrolment_component::authority_row(
                                authority, page.nodes, page.held(&authority.node_id)))
                        }
                    </tbody>
                </table>
            </section>
        }
    }
}
