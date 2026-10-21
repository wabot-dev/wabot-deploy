use hypertext::prelude::*;

use crate::domain::entities::project::Project;
use crate::features::dns::name_servers_component;
use crate::features::dns::texts::{t, word};
use crate::features::dns::zones_response::ZonesResponse;
use crate::shell::layouts::layout;

pub struct ZonesPage<'a> {
    pub project: &'a Project,
    pub zones: &'a ZonesResponse,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ZonesPage<'a>) -> impl Renderable + 'a {
    let zones = page.zones;
    let here = format!("/projects/{}/zones", page.project.slug);
    rsx! {
        (layout::style_tag())
        <h1>(t("DNS"))</h1>
        @if let Some(message) = page.error {
            (layout::error_note(word(message)))
        }

        <section class="card stack">
            <p class="card-label">(t("Zones"))</p>
            @if zones.zones.is_empty() {
                <p class="tile-detail">(t("None yet. A zone is a domain these nodes answer for: its addresses, its mail, everything a resolver asks about it."))</p>
            } @else {
                <table>
                    <thead>
                        <tr>
                            <th>(t("Domain"))</th>
                            <th>(t("Records"))</th>
                            <th>(t("Here"))</th>
                        </tr>
                    </thead>
                    <tbody>
                        @for zone in &zones.zones {
                            <tr>
                                <td><a href=(format!("{here}/{}", zone.name))>(&zone.name)</a></td>
                                <td class="tile-detail">(zone.records.to_string())</td>
                                <td>
                                    @if zone.answered_here {
                                        <span class="badge badge-success"><span class="dot dot-success"></span>(t("Answered"))</span>
                                    } @else {
                                        <span class="badge"><span class="dot dot-info"></span>(t("Not answered yet"))</span>
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            }
        </section>

        @if zones.editable {
            <section class="stack">
                <p class="card-label">(t("Add a zone"))</p>
                <form method="post" action=(&here) class="card stack">
                    <label for="zone-name">(t("Domain"))</label>
                    <input id="zone-name" name="name" type="text" autocomplete="off"
                           placeholder="example.com" required>
                    <p class="field-hint">(t("The domain itself, as it was registered. These nodes answer for it and for every name under it that has a record."))</p>
                    <div class="actions">
                        <button type="submit">(t("Add zone"))</button>
                    </div>
                </form>
            </section>
            (name_servers_component::registrar_card(&zones.name_servers))
        }
    }
}
