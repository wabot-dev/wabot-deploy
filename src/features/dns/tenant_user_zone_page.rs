use hypertext::prelude::*;

use crate::features::dns::texts::{t, word};
use crate::features::dns::zone_response::ZoneResponse;
use crate::features::dns::{name_servers_component, zone_records_component};
use crate::shell::layouts::layout;

pub struct OrganisationZonePage<'a> {
    pub zone: &'a ZoneResponse,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a OrganisationZonePage<'a>) -> impl Renderable + 'a {
    let zone = page.zone;
    let here = format!("/dns/{}", zone.name);
    rsx! {
        (layout::style_tag())
        <h1>(&zone.name)</h1>
        @if let Some(message) = page.error {
            (layout::error_note(word(message)))
        }
        @if !zone.answered_here {
            <p class="tile-detail">(t("No node is answering for it yet. The operator of this node turns DNS on; until then the records are kept and nothing is answered."))</p>
        }

        (name_servers_component::registrar_card(&zone.name_servers))
        (zone_records_component::records_card(&here, zone))

        <section class="card stack">
            <p class="card-label">(t("Delete this zone"))</p>
            <p class="field-hint">(t("Every record goes with it, and this node stops answering for the domain. Resolvers keep what they were told until it expires."))</p>
            <form method="post" action=(format!("{here}/delete"))>
                <button class="btn btn-ghost destructive" type="submit">(t("Delete"))</button>
            </form>
        </section>
    }
}
