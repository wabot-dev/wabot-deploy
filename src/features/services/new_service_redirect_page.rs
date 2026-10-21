use hypertext::prelude::*;

use crate::features::services::texts::{t, word};
use crate::shell::layouts::layout;

pub struct NewRedirectPage<'a> {
    pub action: &'a str,
    pub back: &'a str,
    pub targets: &'a [String],
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a NewRedirectPage<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <div class="stack-sm">
            <h1>(t("Add redirect"))</h1>
        </div>
        @if let Some(message) = page.error {
            (layout::error_note(word(message)))
        }
        <form method="post" action=(page.action) class="card stack">
            <label for="redirect-from">(t("From"))</label>
            <input id="redirect-from" name="hostname" type="text" autocomplete="off" class="mono"
                   placeholder="www.example.com" required autofocus>
            <p class="field-hint">(t("The name to send away. It has to point at this node already, like any hostname: it gets a certificate of its own, so a browser reaches it before it is redirected."))</p>

            <label for="redirect-to">(t("To"))</label>
            <select id="redirect-to" name="to">
                @for target in page.targets {
                    <option value=(target)>(target)</option>
                }
            </select>
            <p class="field-hint">(t("Permanent (301), keeping the path and the query: www.example.com/blog?p=2 lands on example.com/blog?p=2."))</p>

            <div class="actions">
                <button type="submit">(t("Add redirect"))</button>
                <a class="btn btn-ghost" href=(page.back)>(t("Cancel"))</a>
            </div>
        </form>
    }
}
