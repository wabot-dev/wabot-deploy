//! Your account › Activity.

use hypertext::prelude::*;

use crate::domain::values::account_event::AccountEvent;
use crate::features::account::account_tabs_component::tabs;
use crate::features::account::texts::t;
use crate::infra::accounts::events::Event;
use crate::shell::components::account_control::{phone, ICON_LAPTOP, ICON_PHONE};
use crate::shell::layouts::layout;
use crate::shell::texts::sentences;

pub struct ActivityPage<'a> {
    pub username: &'a str,
    pub events: &'a [Event],
}

/// XSS SAFETY: constants, never a value from a request.
const KEY: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="8" cy="15" r="4"/><path d="M11 12l8-8M16 7l2.5 2.5M14 9l2 2"/></svg>"#;
const LOCK: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="5" y="10.5" width="14" height="10" rx="2"/><path d="M8 10.5V7.5a4 4 0 0 1 8 0v3"/></svg>"#;
const LINK: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/></svg>"#;
const OUT: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M10 4H5.5A1.5 1.5 0 0 0 4 5.5v13A1.5 1.5 0 0 0 5.5 20H10M15 16l4-4-4-4M19 12H9"/></svg>"#;

fn icon(event: &Event) -> &'static str {
    match event.kind {
        AccountEvent::PasskeyAdded | AccountEvent::PasskeyRemoved => KEY,
        AccountEvent::PasswordDeleted => LOCK,
        AccountEvent::LinkUsed => LINK,
        AccountEvent::SignedOutEverywhere => OUT,
        _ if phone(&event.detail) => ICON_PHONE,
        _ => ICON_LAPTOP,
    }
}

pub fn render<'a>(page: &'a ActivityPage<'a>) -> impl Renderable + 'a {
    rsx! {
    (layout::style_tag())
    (tabs("activity", page.username))
    <div class="reading stack">
        @if page.events.is_empty() {
            <p class="field-hint">(t("Nothing has happened yet."))</p>
        } @else {
            <div class="items">
                @for event in page.events {
                    <div class=(match event.unread { true => "item unread", false => "item" })>
                        <span class="tile">(hypertext::Raw::dangerously_create(icon(event)))</span>
                        <div>
                            <p class="item-name">
                                (t(event.kind.label()))
                                @if !event.detail.is_empty() {
                                    " · "(sentences::device(&event.detail))
                                }
                            </p>
                            <p class="item-meta">
                                (layout::when(event.at))
                                @if let Some(address) = &event.address {
                                    " · "<span class="mono">(address)</span>
                                }
                            </p>
                        </div>
                        <div class="item-actions">
                            @if event.kind == AccountEvent::RequestRefused {
                                <span class="dot dot-warning"></span>
                            }
                        </div>
                    </div>
                }
            </div>
        }
    </div>
    }
}
