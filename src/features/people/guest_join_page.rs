use hypertext::prelude::*;

use crate::features::people::texts::t;
use crate::infra::accounts::invitations::Invitation;
use crate::shell::components::mark::mark;
use crate::shell::components::passkey::Passkey;
use crate::shell::layouts::layout;

pub struct Join<'a> {
    pub passkey: Option<&'a crate::infra::accounts::ceremonies::Begun>,
    pub heading: &'a str,
    pub token: &'a str,
    pub invitation: Option<&'a Invitation>,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a Join<'a>) -> impl Renderable + 'a {
    let prompt = Passkey {
        create: page.passkey,
        name_field: Some("username"),
        optional: true,
        ..Default::default()
    };
    let form = rsx! {
        <form method="post" action=(format!("/join/{}", page.token)) class="card stack"
              data-wabot-reload>
            <label for="username">(t("Username"))</label>
            <input id="username" name="username" type="text"
                   autocomplete="username webauthn" required autofocus>
            @if page.passkey.is_some() {
                <p class="field-hint">(t("Your device makes a passkey: a fingerprint, a face or a PIN, and nothing to remember."))</p>
            }
            (prompt.fields())
            <div class="actions">
                <button type="submit">(t("Create account"))</button>
            </div>
        </form>
    }
    .render()
    .into_inner();
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
                (prompt.around(form.clone()))
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
