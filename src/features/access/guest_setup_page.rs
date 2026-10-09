//! Claiming a node that has nobody on it yet.

use hypertext::prelude::*;

use crate::views::components::mark::mark;
use crate::views::layouts::layout;
use crate::views::texts::language::t;

pub struct Setup<'a> {
    pub has_token: bool,
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a Setup<'a>) -> impl Renderable + 'a {
    let has_token = page.has_token;
    rsx! {
    (layout::style_tag())
    <main class="shell narrow">
        (mark("Set up this node"))
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }
        @if has_token {
            <p class="tagline">(t("The setup token was printed by `wabot-deploy install`. \
                 It works once, and it expires."))</p>
            <form method="post" action="/setup" class="card stack" data-wabot-reload>
                <label for="setup_token">(t("Setup token"))</label>
                <input id="setup_token" name="setup_token" type="text"
                       autocomplete="off" required>

                <label for="username">(t("Username"))</label>
                <input id="username" name="username" type="text"
                       autocomplete="username" required>

                <label for="password">(t("Password"))</label>
                <input id="password" name="password" type="password"
                       autocomplete="new-password" required>
                <p class="field-hint">(t("At least 12 characters. A phrase beats a puzzle — \
                     this console can start containers on the machine."))</p>

                <div class="actions">
                    <button type="submit">(t("Create administrator"))</button>
                </div>
            </form>
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
