use hypertext::prelude::*;

use crate::domain::values::record_data::RecordKind;
use crate::features::dns::texts::{t, word};
use crate::shell::layouts::layout;

pub struct NewRecordPage<'a> {
    pub zone: &'a str,
    pub action: &'a str,
    pub back: &'a str,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a NewRecordPage<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <div class="stack-sm">
            <h1>(t("Add record"))</h1>
            <p class="tile-detail"><code>(page.zone)</code></p>
        </div>
        @if let Some(message) = page.error {
            (layout::error_note(word(message)))
        }
        <form method="post" action=(page.action) class="card stack">
            <label for="record-name">(t("Name"))</label>
            <input id="record-name" name="name" type="text" autocomplete="off" placeholder="@" autofocus>
            <p class="field-hint">(t("@ is the domain itself. Otherwise what comes before it — www, mg, _dmarc — or the whole name, pasted as your provider shows it."))</p>

            <label for="record-kind">(t("Type"))</label>
            <select id="record-kind" name="kind">
                @for kind in RecordKind::ALL {
                    <option value=(kind.name())>(kind.name())</option>
                }
            </select>

            <label for="record-value">(t("Value"))</label>
            <input id="record-value" name="value" type="text" autocomplete="off" required>
            <p class="field-hint">(t("An address for A and AAAA, a full name for CNAME and MX, the text for TXT — quotes and all, as it was given to you."))</p>

            <label for="record-priority">(t("Priority"))</label>
            <input id="record-priority" name="priority" type="text" inputmode="numeric" autocomplete="off">
            <p class="field-hint">(t("Only for MX and SRV: lower is tried first."))</p>

            <label for="record-ttl">(t("TTL"))</label>
            <input id="record-ttl" name="ttl" type="text" inputmode="numeric" autocomplete="off" placeholder="300">
            <p class="field-hint">(t("Seconds a resolver may keep the answer before asking again."))</p>

            <div class="actions">
                <button type="submit">(t("Add record"))</button>
                <a class="btn btn-ghost" href=(page.back)>(t("Cancel"))</a>
            </div>
        </form>
    }
}
