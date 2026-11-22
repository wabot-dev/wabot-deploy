//! What a one-time link opens. Spending it is a POST, so a prefetcher
//! that follows the link does not use it up.

use hypertext::prelude::*;

use crate::features::access::texts::t;
use crate::infra::accounts::ceremonies::Begun;
use crate::shell::components::mark::mark;
use crate::shell::components::passkey::Passkey;
use crate::shell::layouts::layout;

pub struct LinkPage<'a> {
    pub token: &'a str,
    pub error: Option<&'a str>,
    /// Whose it is, or None when it is not valid.
    pub username: Option<&'a str>,
    /// A device link always trusts the browser; a sign-in link asks.
    pub device: bool,
    pub passkey: Option<&'a Begun>,
}

pub fn render<'a>(page: &'a LinkPage<'a>) -> impl Renderable + 'a {
    let prompt = Passkey {
        create: page.passkey,
        user: page.username,
        optional: true,
        ..Default::default()
    };
    let form = rsx! {
        <form method="post" action=(format!("/link/{}", page.token)) class="stack"
              data-wabot-reload>
            (prompt.fields())
            @if !page.device {
                <label class="check">
                    <input type="checkbox" name="trust" value="on">
                    <span>
                        (t("Trust this device"))
                        <span class="check-hint">(t("Not on a computer somebody else uses."))</span>
                    </span>
                </label>
            }
            @if page.device {
                <button type="submit" class="block">(t("Add this device"))</button>
            } @else {
                <button type="submit" class="block">(t("Sign in"))</button>
            }
        </form>
    }
    .render()
    .into_inner();
    rsx! {
    (layout::style_tag())
    <main class="shell narrow guest">
        (mark(t("Sign in")))
        @if let Some(message) = &page.error {
            (layout::error_note(message))
        }
        @if let Some(username) = page.username {
            <p class="guest-host">(t("Signing in as "))<strong>(username)</strong>"."</p>
            @if page.passkey.is_some() {
                <p class="field-hint">(t("Your device makes a passkey for this console as you sign in."))</p>
            }
            (prompt.around(form.clone()))
        } @else {
            <p>(t("This link is not valid. It may have been used already, or expired."))</p>
            <p><a class="btn block" href="/sign-in">(t("Sign in"))</a></p>
        }
    </main>
    }
}
