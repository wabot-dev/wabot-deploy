//! Signing in: a passkey, and a password only while somebody here still
//! has one.

use hypertext::prelude::*;

use crate::features::access::texts::t;
use crate::infra::accounts::ceremonies::Begun;
use crate::shell::components::mark::mark;
use crate::shell::components::passkey::Passkey;
use crate::shell::layouts::layout;

pub struct SignIn<'a> {
    pub query_error: Option<&'a str>,
    pub passkey: Option<&'a Begun>,
    pub password: bool,
}

pub fn render<'a>(page: &'a SignIn<'a>) -> impl Renderable + 'a {
    let prompt = Passkey {
        get: page.passkey,
        ..Default::default()
    };
    let form = rsx! {
        <form method="post" action="/sign-in/passkey" class="card stack" data-wabot-reload>
            (prompt.fields())
            <div class="actions">
                <button type="submit">(t("Sign in with a passkey"))</button>
            </div>
        </form>
    }
    .render()
    .into_inner();
    let has_passkey = page.passkey.is_some();
    rsx! {
    (layout::style_tag())
    <main class="shell narrow">
        (mark("Sign in"))
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }
        @if has_passkey {
            (prompt.around(form.clone()))
        } @else {
            <section class="card stack">
                <p>(t("Passkeys need this console opened by its name. Without one, ask for a sign-in link."))</p>
            </section>
        }
        @if page.password {
            <form method="post" action="/sign-in" class="card stack" data-wabot-reload>
                <label for="username">(t("Username"))</label>
                <input id="username" name="username" type="text"
                       autocomplete="username" required>

                <label for="password">(t("Password"))</label>
                <input id="password" name="password" type="password"
                       autocomplete="current-password" required>

                <div class="actions">
                    <button type="submit" class="btn btn-secondary">(t("Sign in with a password"))</button>
                </div>
            </form>
        }
    </main>
    }
}
