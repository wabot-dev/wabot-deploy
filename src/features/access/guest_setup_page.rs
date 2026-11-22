//! The page a fresh node opens on.

use hypertext::prelude::*;

use crate::features::access::texts::t;
use crate::infra::accounts::ceremonies::Begun;
use crate::shell::components::mark::mark;
use crate::shell::components::passkey::Passkey;
use crate::shell::layouts::layout;

pub struct Setup<'a> {
    pub has_token: bool,
    pub query_error: Option<&'a str>,
    /// None when the node was opened by its address, which no passkey can
    /// be made for.
    pub passkey: Option<&'a Begun>,
}

pub fn render<'a>(page: &'a Setup<'a>) -> impl Renderable + 'a {
    let prompt = Passkey {
        create: page.passkey,
        name_field: Some("username"),
        optional: true,
        ..Default::default()
    };
    let form = rsx! {
        <form method="post" action="/setup" class="card stack" data-wabot-reload>
            <label for="setup_token">(t("Setup token"))</label>
            <input id="setup_token" name="setup_token" type="text"
                   autocomplete="off" required>

            <label for="username">(t("Username"))</label>
            <input id="username" name="username" type="text"
                   autocomplete="username webauthn" required>
            @if page.passkey.is_some() {
                <p class="field-hint">(t("Your device makes a passkey: a fingerprint, a face or a PIN, and nothing to remember."))</p>
            } @else {
                <p class="field-hint">(t("Opened by its address, this node cannot make passkeys. Sign in again with a link from `wabot-deploy sign-in-link`."))</p>
            }
            (prompt.fields())

            <div class="actions">
                <button type="submit">(t("Create administrator"))</button>
            </div>
        </form>
    }
    .render()
    .into_inner();
    let has_token = page.has_token;
    rsx! {
    (layout::style_tag())
    <main class="shell narrow guest">
        (mark(t("Set up this node")))
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }
        @if has_token {
            <p class="tagline">(t("The setup token was printed by `wabot-deploy install`. \
                 It works once, and it expires."))</p>
            (prompt.around(form.clone()))
        } @else {
            <section class="card stack">
                <p>(t("No setup token is outstanding, so nobody can be created \
                     from here. Issue one on the node:"))</p>
                <pre><code>("wabot-deploy setup-token")</code></pre>
            </section>
        }
    </main>
    }
}
