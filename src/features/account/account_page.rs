//! Your account › Passkeys.

use hypertext::prelude::*;

use crate::features::account::account_tabs_component::tabs;
use crate::features::account::texts::t;
use crate::infra::accounts::ceremonies::Begun;
use crate::infra::accounts::passkeys::Passkey;
use crate::shell::components::passkey::Passkey as Prompt;
use crate::shell::layouts::layout;

pub struct PasskeysPage<'a> {
    pub passkeys: &'a [Passkey],
    /// The host this console was opened by, which a passkey answers for.
    pub host: &'a str,
    pub add: Option<&'a Begun>,
    pub verify: Option<&'a Begun>,
    pub username: &'a str,
    pub password: bool,
    pub error: Option<&'a str>,
    /// The passkey a refused device used.
    pub refused: Option<&'a Passkey>,
}

fn remove_form(page: &PasskeysPage<'_>, passkey: &Passkey, label: &'static str) -> String {
    let prompt = Prompt {
        verify: page.verify,
        ..Default::default()
    };
    let form = rsx! {
        <form method="post" action=(format!("/account/passkeys/{}/delete", passkey.id))>
            (prompt.fields())
            <button type="submit" class="btn btn-secondary btn-sm">(label)</button>
        </form>
    }
    .render()
    .into_inner();
    let rendered = prompt.around(form).render().into_inner();
    rendered
}

pub fn render<'a>(page: &'a PasskeysPage<'a>) -> impl Renderable + 'a {
    let rows: Vec<(String, &Passkey)> = page
        .passkeys
        .iter()
        .map(|passkey| (remove_form(page, passkey, t("Remove")), passkey))
        .collect();
    let refused = page.refused.map(|passkey| {
        (
            remove_form(page, passkey, t("Remove this passkey")),
            passkey,
        )
    });
    let prompt = Prompt {
        create: page.add,
        verify: page.verify,
        user: Some(page.username),
        ..Default::default()
    };
    let add = rsx! {
        <form method="post" action="/account/passkeys" class="stack">
            (prompt.fields())
            <div class="actions">
                <button type="submit">(t("Add a passkey"))</button>
            </div>
        </form>
    }
    .render()
    .into_inner();
    let add = prompt.around(add).render().into_inner();
    rsx! {
    (layout::style_tag())
    (tabs("passkeys"))
    @if let Some(message) = &page.error {
        (layout::error_note(message))
    }
    @if let Some((form, passkey)) = &refused {
        <section class="card stack">
            <p>(t("The device you refused signed in with "))<strong>(&passkey.name)</strong>(t(". Somebody else has it."))</p>
            (hypertext::Raw::dangerously_create(form))
        </section>
    }
    @if page.password {
        <p class="field-hint">(t("You still have a password. Your first passkey deletes it."))</p>
    }
    <section class="stack">
        @if rows.is_empty() {
            <p class="field-hint">(t("No passkeys yet."))</p>
        } @else {
            <table>
                <thead>
                    <tr>
                        <th>(t("Made on"))</th>
                        <th>(t("Added"))</th>
                        <th>(t("Last used"))</th>
                        <th></th>
                    </tr>
                </thead>
                <tbody>
                    @for (form, passkey) in &rows {
                        <tr>
                            <td>
                                (&passkey.name)
                                @if passkey.synced {
                                    " · "<span class="muted">(t("synced"))</span>
                                }
                                @if !page.host.is_empty() && passkey.rp_id != page.host {
                                    " · "<span class="muted">(t("for")) " " (&passkey.rp_id)</span>
                                }
                            </td>
                            <td>(layout::when(passkey.created_at))</td>
                            <td>
                                @if let Some(at) = passkey.last_used_at {
                                    (layout::when(at))
                                } @else {
                                    "—"
                                }
                            </td>
                            <td>(hypertext::Raw::dangerously_create(form))</td>
                        </tr>
                    }
                </tbody>
            </table>
        }
        @if page.add.is_some() {
            (hypertext::Raw::dangerously_create(&add))
        } @else {
            <p class="field-hint">(t("Passkeys need this console opened by its name."))</p>
        }
    </section>
    }
}
