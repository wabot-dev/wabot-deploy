use hypertext::prelude::*;

use crate::domain::entities::project::Project;
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
        <div class="split">
            <h1>(t("DNS"))</h1>
            @if zones.editable {
                <div class="row">
                    <a class="btn" href=(format!("{here}/new"))>(t("Add zone"))</a>
                </div>
            }
        </div>
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
    }
}
