//! Your account › Activity.

use hypertext::prelude::*;

use crate::features::account::account_tabs_component::tabs;
use crate::features::account::texts::t;
use crate::infra::accounts::events::Event;
use crate::shell::layouts::layout;

pub struct ActivityPage<'a> {
    pub events: &'a [Event],
}

pub fn render<'a>(page: &'a ActivityPage<'a>) -> impl Renderable + 'a {
    rsx! {
    (layout::style_tag())
    (tabs("activity"))
    <section class="stack">
        @if page.events.is_empty() {
            <p class="field-hint">(t("Nothing has happened yet."))</p>
        } @else {
            <table>
                <tbody>
                    @for event in page.events {
                        <tr>
                            <td>(t(event.kind.label()))</td>
                            <td>(&event.detail)</td>
                            <td>(event.address.as_deref().unwrap_or(""))</td>
                            <td>(layout::when(event.at))</td>
                        </tr>
                    }
                </tbody>
            </table>
        }
    </section>
    }
}
