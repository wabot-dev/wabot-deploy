use hypertext::prelude::*;

use crate::features::network::node_enrolment_component;
use crate::infra::network::enrolment::Enrolment;
use crate::infra::network::Node;
use crate::web::views::layouts::layout;
use crate::web::views::texts::language::t;

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
        (node_enrolment_component::enrol_card(page.may_enrol))
        (node_enrolment_component::minted_card(page.enrolments, page.nodes, page.now))
    }
}
