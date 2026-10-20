use hypertext::prelude::*;

use crate::features::dns::texts::{t, word};
use crate::features::dns::zones_response::{ZoneOwner, ZonesResponse};
use crate::features::dns::{dns_status_component, own_names_component};
use crate::shell::layouts::layout;

pub struct ZonesPage<'a> {
    pub zones: &'a ZonesResponse,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ZonesPage<'a>) -> impl Renderable + 'a {
    let zones = page.zones;
    rsx! {
        (layout::style_tag())
        <h1>(t("DNS"))</h1>
        @if let Some(message) = page.error {
            (layout::error_note(word(message)))
        }
        (dns_status_component::status_card(
            zones.provides_dns,
            zones.addressed,
            zones.zones.iter().any(|zone| zone.answered_here),
            &zones.listening,
        ))
        (own_names_component::names_card(zones))

        <section class="card stack">
            <p class="card-label">(t("Zones"))</p>
            @if zones.zones.is_empty() {
                <p class="tile-detail">(t("None yet. A zone is a domain this node answers for: its addresses, its mail, everything a resolver asks about it."))</p>
            } @else {
                <table>
                    <thead>
                        <tr>
                            <th>(t("Domain"))</th>
                            <th>(t("Whose"))</th>
                            <th>(t("Records"))</th>
                            <th>(t("Here"))</th>
                        </tr>
                    </thead>
                    <tbody>
                        @for zone in &zones.zones {
                            <tr>
                                <td><a href=(format!("/dns/{}", zone.name))>(&zone.name)</a></td>
                                <td class="tile-detail">
                                    @match &zone.owner {
                                        ZoneOwner::ThisNode => { (t("This node")) }
                                        ZoneOwner::Organisation(name) => { (name) }
                                        ZoneOwner::SentBy(node) => { (t("Sent by")) " " (node) }
                                    }
                                </td>
                                <td class="tile-detail">(zone.records.to_string())</td>
                                <td>
                                    @if zone.answered_here {
                                        <span class="badge badge-success"><span class="dot dot-success"></span>(t("Answered"))</span>
                                    } @else {
                                        <span class="badge"><span class="dot dot-info"></span>(t("Not here"))</span>
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            }
        </section>

        <section class="stack">
            <p class="card-label">(t("Add a zone"))</p>
            <form method="post" action="/dns/zones" class="card stack">
                <label for="zone-name">(t("Domain"))</label>
                <input id="zone-name" name="name" type="text" autocomplete="off"
                       placeholder="example.com" required>
                <p class="field-hint">(t("The domain itself, as it was registered. This node answers for it and for every name under it that has a record."))</p>
                <div class="actions">
                    <button type="submit">(t("Add zone"))</button>
                </div>
            </form>
        </section>
    }
}
