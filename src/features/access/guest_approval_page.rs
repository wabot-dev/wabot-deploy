//! A device nobody trusts, waiting for one that is trusted.

use hypertext::prelude::*;

use crate::features::access::texts::t;
use crate::shell::components::mark::mark;
use crate::shell::layouts::layout;

pub struct Approval<'a> {
    pub error: Option<&'a str>,
    /// Whether this browser has a request still open.
    pub open: bool,
}

pub fn render<'a>(page: &'a Approval<'a>) -> impl Renderable + 'a {
    rsx! {
    (layout::style_tag())
    <main class="shell narrow">
        (mark("Sign in"))
        @if let Some(message) = &page.error {
            (layout::error_note(message))
        }
        @if page.open {
            <p class="tagline">(t("Open this console on a device you trust and choose It was me. Type the six digits it shows."))</p>
            <form method="post" action="/sign-in/approval" class="card stack" data-wabot-reload>
                <label for="code">(t("Code"))</label>
                <input id="code" name="code" type="text" inputmode="numeric"
                       pattern="[0-9]{6}" maxlength="6" autocomplete="one-time-code"
                       required autofocus>
                <label class="check">
                    <input type="checkbox" name="trust" value="on">
                    (t("Trust this device"))
                </label>
                <div class="actions">
                    <button type="submit">(t("Sign in"))</button>
                </div>
            </form>
        } @else {
            <section class="card stack">
                <p>(t("That request is closed."))</p>
                <p><a href="/sign-in">(t("Sign in again"))</a></p>
            </section>
        }
    </main>
    }
}
