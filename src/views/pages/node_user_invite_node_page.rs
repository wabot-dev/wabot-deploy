use hypertext::prelude::*;

use crate::network::enrolment::Enrolment;
use crate::network::Node;
use crate::views::components::node_enrolment;
use crate::views::layouts::layout;
use crate::views::texts::language::t;

pub struct InviteNode<'a> {
    pub nodes: &'a [Node],
    pub enrolments: &'a [Enrolment],
    /// **Spent on read**: the token exists in clear for exactly this one
    /// page load. A query parameter would put it in the address bar, the
    /// history and every refresh.
    pub revealed: Option<&'a str>,
    pub may_enrol: bool,
    pub now: i64,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a InviteNode<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <h1>(t("Invite a node"))</h1>
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }
        @if let Some(token) = page.revealed {
            <section class="card stack">
                <p class="card-label">(t("Run this on the other node"))</p>
                <pre><code>("wabot-deploy join ")(token)</code></pre>
                <p class="field-hint">(t("It works once and expires in 24 hours. This node will not \
                     show it again — what is stored is its hash. The other \
                     machine has to be a node already: install it there first, \
                     then join."))</p>
            </section>
        }
        (node_enrolment::enrol_card(page.may_enrol))
        (node_enrolment::minted_card(page.enrolments, page.nodes, page.now))
    }
}
