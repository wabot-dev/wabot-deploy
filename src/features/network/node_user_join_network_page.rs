use hypertext::prelude::*;

use crate::features::network::node_enrolment_component;
use crate::network::token::JoinToken;
use crate::views::layouts::layout;
use crate::views::texts::language::t;

pub struct JoinNetwork<'a> {
    /// A token pasted a moment ago, with what it asks for. **Held for
    /// exactly this render**: terms shown after committing are a consent
    /// screen for a decision already made.
    pub reviewing: Option<&'a (String, JoinToken)>,
    pub joined: Option<&'a str>,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a JoinNetwork<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <h1>(t("Join a network"))</h1>
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }
        @if let Some((token, terms)) = page.reviewing {
            (node_enrolment_component::review_card(token, terms))
        }
        (node_enrolment_component::join_card(page.joined))
    }
}
