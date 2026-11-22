//! The logo and a heading — what a page with no header bar opens with.

use hypertext::prelude::*;

/// The logo and a heading — what a page with no header bar opens with.
pub fn mark(heading: &str) -> impl Renderable + '_ {
    rsx! {
        <header class="mark">
            <span class="mark-brand">
                <img
                    src=(format!("{}/wabot-logo.png", crate::shell::assets::MOUNT))
                    alt="Wabot" width="22" height="22">
                ("wabot-deploy")
            </span>
            <h1>(heading)</h1>
        </header>
    }
}
