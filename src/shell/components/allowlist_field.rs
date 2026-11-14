//! The field every allowlist is written in, and what it says about the
//! one saved.
//!
//! A textarea rather than rows with buttons: it submits with scripting
//! off, a list is pasted as often as it is typed, and the note beside each
//! address is the part a person reads when deciding what to remove.

use hypertext::prelude::*;

use crate::domain::values::allowlist::Allowlist;
use crate::shell::texts::language::t;

/// The field for a list being made, which has nothing to say yet.
pub fn field<'a>(
    id: &'a str,
    name: &'a str,
    label: &'a str,
    list: &'a Allowlist,
) -> impl Renderable + 'a {
    rsx! {
        <label for=(id)>(label)</label>
        (textarea(id, name, list))
    }
}

/// The field for a saved list, which says first whom it lets in.
pub fn saved<'a>(
    id: &'a str,
    name: &'a str,
    label: &'a str,
    list: &'a Allowlist,
) -> impl Renderable + 'a {
    rsx! {
        <label for=(id)>(label)</label>
        (state(list))
        (textarea(id, name, list))
    }
}

fn textarea<'a>(id: &'a str, name: &'a str, list: &'a Allowlist) -> impl Renderable + 'a {
    rsx! {
        <textarea id=(id) name=(name) autocomplete="off" rows="4" class="mono"
                  placeholder="203.0.113.5 office">(list.text())</textarea>
        <p class="field-hint">(t("One address or range per line, and a note after it if you like. \
             Empty lets nobody in. Everyone is 0.0.0.0/0 and ::/0, on two lines."))</p>
    }
}

/// Who the saved list lets in, in a sentence.
pub fn state<'a>(list: &'a Allowlist) -> impl Renderable + 'a {
    rsx! {
        @if list.is_empty() {
            <p class="note">(t("Nobody can connect: the list is empty."))</p>
        } @else if list.is_everyone() {
            <p class="field-hint">(t("Anybody can connect."))</p>
        } @else {
            <p class="field-hint">(t("Only the addresses below can connect."))</p>
        }
    }
}

/// What the edge never refuses, said once where somebody is writing a list.
pub fn always_let_in() -> impl Renderable {
    rsx! {
        <p class="field-hint">(t("This machine itself is always let in, whatever the list says."))</p>
    }
}
