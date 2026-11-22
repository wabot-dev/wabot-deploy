//! Signing in: a passkey, and a password only while somebody here still
//! has one.
//!
//! **The username field is for the browser.** With
//! `autocomplete="username webauthn"` it offers this console's passkeys the
//! moment it is touched, which is what worked best in FIDO's research;
//! the button does the same with nothing typed, because every passkey
//! here is discoverable. While a password exists the field is also the
//! password's, through `form=`, so one field serves both doors.

use hypertext::prelude::*;

use crate::features::access::texts::t;
use crate::infra::accounts::ceremonies::Begun;
use crate::shell::components::mark::mark;
use crate::shell::components::passkey::Passkey;
use crate::shell::layouts::layout;

pub struct SignIn<'a> {
    pub query_error: Option<&'a str>,
    /// The name the page was opened by, which a passkey answers for.
    pub host: &'a str,
    pub passkey: Option<&'a Begun>,
    pub password: bool,
    /// Where this console answers by name, when it was opened by its
    /// address and has one.
    pub named: Option<&'a str>,
}

/// XSS SAFETY: a constant.
const PASSKEY: &str = r#"<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="9" cy="7.5" r="3.5"/><path d="M2.5 20c0-3.6 2.9-6.5 6.5-6.5 1.3 0 2.5.4 3.5 1"/><circle cx="17.5" cy="13.5" r="2.5"/><path d="M17.5 16v5.5M17.5 19.5h2"/></svg>"#;

pub fn render<'a>(page: &'a SignIn<'a>) -> impl Renderable + 'a {
    let prompt = Passkey {
        get: page.passkey,
        autofill: true,
        ..Default::default()
    };
    let password = page.password;
    let block = rsx! {
        @if password {
            <form id="password-form" method="post" action="/sign-in" data-wabot-reload></form>
        }
        <form method="post" action="/sign-in/passkey" class="stack" data-passkey data-wabot-reload>
            (prompt.fields())
            <label for="username">(t("Username"))</label>
            @if password {
                <input id="username" name="username" type="text" form="password-form"
                       autocomplete="username webauthn" autocapitalize="none" spellcheck="false">
            } @else {
                <input id="username" name="username" type="text"
                       autocomplete="username webauthn" autocapitalize="none" spellcheck="false">
            }
            <button type="submit" class="block with-icon">
                (hypertext::Raw::dangerously_create(PASSKEY))
                (t("Sign in with a passkey"))
            </button>
        </form>
        @if password {
            <details class="other-way">
                <summary>(t("Sign in with a password"))</summary>
                <div class="stack">
                    <label for="password">(t("Password"))</label>
                    <input id="password" name="password" type="password" form="password-form"
                           autocomplete="current-password">
                    <button type="submit" form="password-form" class="btn btn-secondary block">
                        (t("Sign in with a password"))
                    </button>
                </div>
            </details>
        }
    }
    .render()
    .into_inner();
    let has_passkey = page.passkey.is_some();
    rsx! {
    (layout::style_tag())
    <main class="shell narrow guest">
        (mark(t("Sign in")))
        <p class="guest-host">(page.host)</p>
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }
        @if has_passkey {
            (prompt.around(block.clone()))
        } @else {
            <section class="stack">
                <p>(t("Passkeys work only with this console's name."))</p>
                @if let Some(named) = page.named {
                    <a class="btn block" href=(format!("{named}/sign-in"))>(t("Open")) " " (named.trim_start_matches("https://"))</a>
                }
                <p class="item-meta">(t("Without it, sign in with a link from the node:"))</p>
                <pre><code>("wabot-deploy sign-in-link <username>")</code></pre>
            </section>
            @if password {
                <form method="post" action="/sign-in" class="stack" data-wabot-reload>
                    <label for="username">(t("Username"))</label>
                    <input id="username" name="username" type="text" autocomplete="username" required>
                    <label for="password">(t("Password"))</label>
                    <input id="password" name="password" type="password"
                           autocomplete="current-password" required>
                    <button type="submit" class="btn btn-secondary block">(t("Sign in with a password"))</button>
                </form>
            }
        }
        <p class="guest-foot">(t("Lost your device? Whoever runs this console can give you a sign-in link."))</p>
    </main>
    }
}
