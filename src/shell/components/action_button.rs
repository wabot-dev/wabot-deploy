//! A button that acts, and the attribute that hides what does not apply.

use hypertext::prelude::*;

/// The two icons, inlined rather than fetched: the design system names
/// Lucide for the rare icon, and this console cannot reach a CDN — the
/// same reason the fonts are vendored.
///
/// Raw because `rsx!` validates element names and does not know SVG's.
///
/// XSS SAFETY: two constants in this file, never a value from a request.
pub const PLAY: &str = r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="6 3 20 12 6 21 6 3"/></svg>"#;
pub const STOP: &str = r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>"#;

/// Whether a row's control is the one that applies.
///
/// A class rather than the `hidden` attribute, because `hidden` is
/// boolean *by presence*: `rsx!` renders `hidden=(false)` as
/// `hidden="false"`, which hides the element just as thoroughly. Both
/// controls were hidden on first paint and only appeared when the
/// stream's first message set the DOM property. `class` has no such
/// trap — an empty one matches nothing.
pub fn shown(applies: bool) -> &'static str {
    if applies {
        ""
    } else {
        "is-hidden"
    }
}

pub fn action_button(label: &'static str, icon: &'static str, busy: bool) -> impl Renderable {
    rsx! {
        @if busy {
            <button class="btn btn-secondary btn-sm btn-icon" type="submit"
                    title=(label) aria-label=(label) disabled>
                (hypertext::Raw::dangerously_create(icon))
            </button>
        } @else {
            <button class="btn btn-secondary btn-sm btn-icon" type="submit"
                    title=(label) aria-label=(label)>
                (hypertext::Raw::dangerously_create(icon))
            </button>
        }
    }
}
