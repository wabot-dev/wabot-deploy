use hypertext::prelude::*;

use crate::domain::entities::project::Project;
use crate::features::dns::texts::{t, word};
use crate::features::dns::zone_response::ZoneResponse;
use crate::features::dns::{
    delegation_component, name_servers_component, zone_records_component, zone_servers_component,
};
use crate::shell::layouts::layout;

pub struct ZonePage<'a> {
    pub project: &'a Project,
    pub zone: &'a ZoneResponse,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ZonePage<'a>) -> impl Renderable + 'a {
    let zone = page.zone;
    let here = format!("/projects/{}/zones/{}", page.project.slug, zone.name);
    rsx! {
        (layout::style_tag())
        <h1>(&zone.name)</h1>
        @if let Some(message) = page.error {
            (layout::error_note(word(message)))
        }
        <p class="tile-detail">
            @if !zone.provides_dns {
                (t("No node is answering for it yet. The operator of this node turns DNS on; until then the records are kept and nothing is answered."))
            } @else if zone.answered_here {
                (t("This node is answering for it."))
            } @else {
                (t("This node is not one of its name servers."))
            }
        </p>

        @if !zone.servers.is_empty() {
            (zone_servers_component::servers_card(&here, zone))
        }
        (name_servers_component::registrar_card(&zone.name_servers))
        @if zone.editable {
            (delegation_component::delegation_card(&here, zone))
        }
        (zone_records_component::records_card(&here, zone))

        @if zone.editable {
            <section class="card stack">
                <p class="card-label">(t("Delete this zone"))</p>
                <p class="field-hint">(t("Every record goes with it, and these nodes stop answering for the domain. Resolvers keep what they were told until it expires."))</p>
                <form method="post" action=(format!("{here}/delete"))>
                    <button class="btn btn-ghost destructive" type="submit">(t("Delete"))</button>
                </form>
            </section>
        }
    }
}
