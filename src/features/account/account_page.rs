//! Your account › Passkeys.

use hypertext::prelude::*;

use crate::domain::values::passkey_provider;
use crate::features::account::account_tabs_component::tabs;
use crate::features::account::texts::t;
use crate::infra::accounts::ceremonies::Begun;
use crate::infra::accounts::passkeys::Passkey;
use crate::shell::components::passkey::{Known, Passkey as Prompt};
use crate::shell::layouts::layout;
use crate::shell::texts::sentences;

pub struct PasskeysPage<'a> {
    pub passkeys: &'a [Passkey],
    /// The host this console was opened by, which a passkey answers for.
    pub host: &'a str,
    pub add: Option<&'a Begun>,
    pub verify: Option<&'a Begun>,
    pub username: &'a str,
    /// The user handle every passkey of this account carries.
    pub account_id: &'a str,
    pub password: bool,
    pub error: Option<&'a str>,
    /// The passkey a refused device used.
    pub refused: Option<&'a Passkey>,
}

/// XSS SAFETY: constants, never a value from a request.
const KEY: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="8" cy="15" r="4"/><path d="M11 12l8-8M16 7l2.5 2.5M14 9l2 2"/></svg>"#;
const PASSKEY: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="9" cy="7.5" r="3.5"/><path d="M2.5 20c0-3.6 2.9-6.5 6.5-6.5 1.3 0 2.5.4 3.5 1"/><circle cx="17.5" cy="13.5" r="2.5"/><path d="M17.5 16v5.5M17.5 19.5h2"/></svg>"#;
const PEN: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 20h4L19 9a2.8 2.8 0 0 0-4-4L4 16z"/></svg>"#;
const CROSS: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18"/></svg>"#;

/// **A cross, not a bin.** Taking a passkey off the account leaves it in
/// the password manager that holds it, so a bin would promise a deletion
/// this console cannot make.
fn remove_form(
    page: &PasskeysPage<'_>,
    passkey: &Passkey,
    labelled: Option<&'static str>,
) -> String {
    let prompt = Prompt {
        verify: page.verify,
        ..Default::default()
    };
    let form = rsx! {
        <form method="post" action=(format!("/account/passkeys/{}/delete", passkey.id))>
            (prompt.fields())
            @if let Some(label) = labelled {
                <button type="submit" class="btn btn-danger btn-sm">(label)</button>
            } @else {
                <button type="submit" class="btn btn-ghost btn-icon" title=(t("Remove passkey"))
                        aria-label=(t("Remove passkey"))>
                    (hypertext::Raw::dangerously_create(CROSS))
                </button>
            }
        </form>
    }
    .render()
    .into_inner();
    let rendered = prompt.around(form).render().into_inner();
    rendered
}

fn tile(passkey: &Passkey) -> String {
    match passkey.provider() {
        Some(name) if !passkey.security_key() => passkey_provider::initials(name),
        _ => String::new(),
    }
}

fn kind(passkey: &Passkey) -> &'static str {
    match (passkey.synced, passkey.security_key()) {
        (true, _) => t("Synced"),
        (false, true) => t("Security key"),
        (false, false) => t("Not synced"),
    }
}

fn row(page: &PasskeysPage<'_>, passkey: &Passkey, last: bool) -> String {
    let dialog = format!("rename-{}", passkey.id);
    let confirm = format!("remove-{}", passkey.id);
    let initials = tile(passkey);
    let other_host = !page.host.is_empty() && passkey.rp_id != page.host;
    let used = match passkey.last_used_at {
        Some(at) => sentences::last_used(&layout::when(at)),
        None => t("not used yet").to_string(),
    };
    let meta = format!(
        "{} · {} · {used}",
        kind(passkey),
        sentences::added_in(
            &layout::when(passkey.created_at),
            &sentences::device(&passkey.name)
        ),
    );
    let placeholder = passkey.provider().unwrap_or(&passkey.name).to_string();
    let remove = match last {
        true => None,
        false => Some(remove_form(page, passkey, None)),
    };
    let remove_anyway = match last {
        true => Some(remove_form(page, passkey, Some(t("Remove anyway")))),
        false => None,
    };
    rsx! {
        <div class="item">
            <span class="tile">
                @if initials.is_empty() {
                    (hypertext::Raw::dangerously_create(KEY))
                } @else {
                    (initials)
                }
            </span>
            <div>
                <p class="item-name">
                    (passkey.title())
                    @if other_host {
                        <span class="badge">(t("for")) " " (&passkey.rp_id)</span>
                    }
                </p>
                <p class="item-meta">(meta)</p>
            </div>
            <div class="item-actions">
                <button type="button" class="btn btn-ghost btn-icon" popovertarget=(&dialog)
                        title=(t("Rename")) aria-label=(t("Rename"))>
                    (hypertext::Raw::dangerously_create(PEN))
                </button>
                @if let Some(form) = &remove {
                    (hypertext::Raw::dangerously_create(form))
                } @else {
                    <button type="button" class="btn btn-ghost btn-icon" popovertarget=(&confirm)
                            title=(t("Remove passkey")) aria-label=(t("Remove passkey"))>
                        (hypertext::Raw::dangerously_create(CROSS))
                    </button>
                }
            </div>
            <div popover id=(&dialog) class="dialog">
                <form method="post" action=(format!("/account/passkeys/{}/rename", passkey.id))
                      class="stack">
                    <label for=(format!("{dialog}-label"))>(t("Name"))</label>
                    <input id=(format!("{dialog}-label")) name="label" type="text" maxlength="60"
                           autocomplete="off" value=(passkey.label.as_deref().unwrap_or(""))
                           placeholder=(&placeholder)>
                    <p class="field-hint">(t("Empty goes back to where it lives."))</p>
                    <div class="actions">
                        <button type="submit">(t("Save"))</button>
                        <button type="button" class="btn btn-ghost" popovertarget=(&dialog)
                                popovertargetaction="hide">(t("Cancel"))</button>
                    </div>
                </form>
            </div>
            @if let Some(form) = &remove_anyway {
                <div popover id=(&confirm) class="dialog">
                    <p>(t("It is your only passkey. Without it, signing in needs a sign-in link."))</p>
                    <div class="actions">
                        (hypertext::Raw::dangerously_create(form))
                        <button type="button" class="btn btn-ghost btn-sm" popovertarget=(&confirm)
                                popovertargetaction="hide">(t("Cancel"))</button>
                    </div>
                </div>
            }
        </div>
    }
    .render()
    .into_inner()
}

pub fn render<'a>(page: &'a PasskeysPage<'a>) -> impl Renderable + 'a {
    let last = page.passkeys.len() == 1;
    let rows: Vec<String> = page
        .passkeys
        .iter()
        .map(|passkey| row(page, passkey, last))
        .collect();
    let refused = page.refused.map(|passkey| {
        (
            remove_form(page, passkey, Some(t("Remove this passkey"))),
            passkey,
        )
    });
    let known: Vec<&str> = page
        .passkeys
        .iter()
        .filter(|passkey| passkey.rp_id == page.host)
        .map(|passkey| passkey.id.as_str())
        .collect();
    let prompt = Prompt {
        create: page.add,
        verify: page.verify,
        user: Some(page.username),
        known: Some(Known {
            host: page.host,
            handle: page.account_id,
            ids: &known,
        }),
        ..Default::default()
    };
    let empty = page.passkeys.is_empty();
    let add = rsx! {
        <form method="post" action="/account/passkeys">
            (prompt.fields())
            @if empty {
                <button type="submit">(t("Make a passkey"))</button>
            } @else {
                <button type="submit" class="btn btn-sm">(t("Add a passkey"))</button>
            }
        </form>
    }
    .render()
    .into_inner();
    let add = prompt.around(add).render().into_inner();
    let count = sentences::passkeys(page.passkeys.len());
    rsx! {
    (layout::style_tag())
    (tabs("passkeys", page.username))
    <div class="reading stack">
        @if let Some(message) = &page.error {
            (layout::error_note(message))
        }
        @if let Some((form, passkey)) = &refused {
            <div class="warning-bar">
                <p>(t("The device you refused signed in with "))<strong>(passkey.title())</strong>(t(". Somebody else has it."))</p>
                (hypertext::Raw::dangerously_create(form))
            </div>
        }
        @if empty {
            <section class="hero">
                <span class="tile tile-brand">(hypertext::Raw::dangerously_create(PASSKEY))</span>
                <div class="stack-sm">
                    <h2>(t("Sign in without a password"))</h2>
                    <p class="item-meta">(t("Your device makes a passkey with your fingerprint, face or screen lock."))</p>
                    @if page.password {
                        <p class="item-meta">(t("Your first passkey deletes your password."))</p>
                    }
                    @if page.add.is_some() {
                        (hypertext::Raw::dangerously_create(&add))
                    } @else {
                        <p class="item-meta">(t("Passkeys need this console opened by its name."))</p>
                    }
                </div>
            </section>
        } @else {
            <div class="list-head">
                <p class="card-label">(count)</p>
                @if page.add.is_some() {
                    (hypertext::Raw::dangerously_create(&add))
                }
            </div>
            <div class="items">
                @for row in &rows {
                    (hypertext::Raw::dangerously_create(row))
                }
            </div>
            @if page.add.is_none() {
                <p class="field-hint">(t("Passkeys need this console opened by its name."))</p>
            }
        }
    </div>
    }
}
