//! The logo and a heading — what a page with no header bar opens with.

use hypertext::prelude::*;

/// The logo and a heading — what a page with no header bar opens with.
pub fn mark(heading: &str) -> impl Renderable + '_ {
    rsx! {
        <header class="mark">
            <img
                src=(format!("{}/wabot-logo.png", crate::web::console::assets::MOUNT))
                alt="Wabot" width="36" height="36">
            <div class="stack-sm">
                <h1>(heading)</h1>
                <p class="tagline">("wabot-deploy")</p>
            </div>
        </header>
    }
}
