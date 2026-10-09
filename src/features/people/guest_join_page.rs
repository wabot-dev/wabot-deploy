use hypertext::prelude::*;

use crate::features::people::texts::t;
use crate::infra::accounts::invitations::Invitation;
use crate::web::views::components::mark::mark;
use crate::web::views::layouts::layout;

pub struct Join<'a> {
    pub heading: &'a str,
    pub token: &'a str,
    pub invitation: Option<&'a Invitation>,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a Join<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <main class="shell narrow">
            (mark(page.heading))
            @if let Some(message) = &page.error {
                (layout::error_note(message))
            }
            @if let Some(invitation) = &page.invitation {
                <p class="tagline">
                    (t("You were invited as "))(invitation.node_role.label().to_lowercase())(".")
                </p>
                <form method="post" action=(format!("/join/{}", page.token)) class="card stack"
                      data-wabot-reload>
                    <label for="username">(t("Username"))</label>
                    <input id="username" name="username" type="text"
                           autocomplete="username" required autofocus>

                    <label for="password">(t("Password"))</label>
                    <input id="password" name="password" type="password"
                           autocomplete="new-password" required>
                    <p class="field-hint">
                        (t("At least 12 characters. Nobody here will ever see it."))
                    </p>

                    <div class="actions">
                        <button type="submit">(t("Create account"))</button>
                    </div>
                </form>
            } @else {
                <section class="card stack">
                    <p>(t("This invitation is not valid. It may have been used already, \
                         withdrawn, or expired — they last seven days."))</p>
                    <p class="field-hint">(t("Ask whoever invited you for another."))</p>
                </section>
            }
        </main>
    }
}
