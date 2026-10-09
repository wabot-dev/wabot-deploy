//! Signing in.

use hypertext::prelude::*;

use crate::features::access::texts::t;
use crate::shell::components::mark::mark;
use crate::shell::layouts::layout;

pub struct SignIn<'a> {
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a SignIn<'a>) -> impl Renderable + 'a {
    rsx! {
    (layout::style_tag())
    <main class="shell narrow">
        (mark("Sign in"))
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }
        <form method="post" action="/sign-in" class="card stack" data-wabot-reload>
            <label for="username">(t("Username"))</label>
            <input id="username" name="username" type="text"
                   autocomplete="username" required>

            <label for="password">(t("Password"))</label>
            <input id="password" name="password" type="password"
                   autocomplete="current-password" required>

            <div class="actions">
                <button type="submit">(t("Sign in"))</button>
            </div>
        </form>
    </main>
    }
}
