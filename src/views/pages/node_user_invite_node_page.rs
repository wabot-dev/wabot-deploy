use hypertext::prelude::*;

use crate::network::enrolment::Enrolment;
use crate::network::Node;
use crate::views::components::node_enrolment;
use crate::views::layouts::layout;
use crate::views::texts::language::t;

pub struct InviteNode<'a> {
    pub nodes: &'a [Node],
    pub enrolments: &'a [Enrolment],
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
        (node_enrolment::enrol_card(page.may_enrol))
        (node_enrolment::minted_card(page.enrolments, page.nodes, page.now))
    }
}
