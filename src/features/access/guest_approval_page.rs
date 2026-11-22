//! A device nobody trusts, waiting for one that is trusted.

use hypertext::prelude::*;

use crate::features::access::texts::t;
use crate::shell::components::countdown::countdown;
use crate::shell::components::mark::mark;
use crate::shell::layouts::layout;
use crate::shell::texts::sentences;

pub struct Approval<'a> {
    pub error: Option<&'a str>,
    /// Whose account it asks for, and how long it has left, while this
    /// browser has a request open.
    pub open: Option<(&'a str, i64)>,
}

pub fn render<'a>(page: &'a Approval<'a>) -> impl Renderable + 'a {
    rsx! {
    (layout::style_tag())
    @if let Some((username, left)) = page.open {
        <main class="shell narrow guest">
            (mark(t("Confirm it from another device")))
            <p class="guest-host">(sentences::new_to(username))</p>
            @if let Some(message) = &page.error {
                (layout::error_note(message))
            }
            <ol class="steps">
                <li>(t("Open this console on a device you trust."))</li>
                <li>(t("Press the account icon there, then It was me."))</li>
                <li>(t("Type the six digits it shows."))</li>
            </ol>
            <form method="post" action="/sign-in/approval" class="stack" data-wabot-reload>
                <label for="code">(t("Code"))</label>
                <input id="code" name="code" type="text" inputmode="numeric" class="code-field"
                       pattern="[0-9 ]{6,7}" maxlength="7" autocomplete="one-time-code"
                       required autofocus>
                <label class="check">
                    <input type="checkbox" name="trust" value="on">
                    <span>
                        (t("Trust this device"))
                        <span class="check-hint">(t("Not on a computer somebody else uses."))</span>
                    </span>
                </label>
                <button type="submit" class="block">(t("Sign in"))</button>
            </form>
            <p class="guest-foot">
                (countdown(left))" · "<a href="/sign-in">(t("Cancel"))</a>
            </p>
        </main>
    } @else {
        <main class="shell narrow guest">
            (mark(t("Sign in")))
            @if let Some(message) = &page.error {
                (layout::error_note(message))
            }
            <p>(t("That request is closed."))</p>
            <p><a class="btn block" href="/sign-in">(t("Sign in again"))</a></p>
        </main>
    }
    }
}
