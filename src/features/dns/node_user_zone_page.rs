use hypertext::prelude::*;

use crate::features::dns::texts::{t, word};
use crate::features::dns::zone_response::ZoneResponse;
use crate::features::dns::zones_response::ZoneOwner;
use crate::features::dns::{
    name_servers_component, zone_records_component, zone_servers_component,
};
use crate::shell::layouts::layout;

pub struct ZonePage<'a> {
    pub zone: &'a ZoneResponse,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ZonePage<'a>) -> impl Renderable + 'a {
    let zone = page.zone;
    let here = format!("/dns/{}", zone.name);
    rsx! {
        (layout::style_tag())
        <h1>(&zone.name)</h1>
        @if let Some(message) = page.error {
            (layout::error_note(word(message)))
        }
        <p class="tile-detail">
            @match &zone.owner {
                ZoneOwner::ThisNode => { (t("This node's own zone.")) }
                ZoneOwner::Organisation(name) => { (t("The zone of the organisation")) " " <strong>(name)</strong> }
                ZoneOwner::SentBy(node) => { (t("Sent by")) " " <strong>(node)</strong> }
            }
            " "
            @if !zone.provides_dns {
                (t("This node does not answer DNS yet: turn on Answer DNS in its settings."))
            } @else if zone.answered_here {
                (t("This node is answering for it."))
            } @else {
                (t("This node is not one of its name servers."))
            }
        </p>

        @if zone.editable {
            (zone_servers_component::servers_card(&here, zone))
        }
        (name_servers_component::registrar_card(&zone.name_servers))
        (zone_records_component::records_card(&here, zone))

        <section class="card stack">
            <p class="card-label">
                @if zone.editable { (t("Delete this zone")) } @else { (t("Stop answering for it")) }
            </p>
            <p class="field-hint">
                @if zone.editable {
                    (t("Every record goes with it, and this node stops answering for the domain. Resolvers keep what they were told until it expires."))
                } @else {
                    (t("This node stops answering for it and forgets the records. The node that sent it is not told, and sends it again the next time it changes — to stop that, withdraw what this node lets it do."))
                }
            </p>
            <form method="post" action=(format!("{here}/delete"))>
                <button class="btn btn-ghost destructive" type="submit">
                    @if zone.editable { (t("Delete")) } @else { (t("Stop answering")) }
                </button>
            </form>
        </section>
    }
}
