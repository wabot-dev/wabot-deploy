use hypertext::prelude::*;

use crate::features::dns::name_servers_component;
use crate::features::dns::texts::{t, word};
use crate::features::dns::zone_response::NameServer;
use crate::shell::layouts::layout;

pub struct NewZonePage<'a> {
    pub action: &'a str,
    pub back: &'a str,
    pub name_servers: &'a [NameServer],
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a NewZonePage<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <div class="stack-sm">
            <h1>(t("Add zone"))</h1>
        </div>
        @if let Some(message) = page.error {
            (layout::error_note(word(message)))
        }
        <form method="post" action=(page.action) class="card stack">
            <label for="zone-name">(t("Domain"))</label>
            <input id="zone-name" name="name" type="text" autocomplete="off"
                   placeholder="example.com" required autofocus>
            <p class="field-hint">(t("The domain itself, as it was registered. These nodes answer for it and for every name under it that has a record."))</p>
            <div class="actions">
                <button type="submit">(t("Add zone"))</button>
                <a class="btn btn-ghost" href=(page.back)>(t("Cancel"))</a>
            </div>
        </form>
        (name_servers_component::registrar_card(page.name_servers))
    }
}
