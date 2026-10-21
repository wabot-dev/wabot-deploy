use hypertext::prelude::*;

use crate::features::dns::texts::t;
use crate::features::dns::zone_response::ZoneResponse;

/// `here` is the zone's own path in whichever console draws it.
pub fn records_card<'a>(here: &'a str, zone: &'a ZoneResponse) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("Records"))</p>
            @if zone.records.is_empty() {
                <p class="tile-detail">(t("None yet. The name servers and the start of authority are written by the node and are not listed here."))</p>
            } @else {
                <table>
                    <thead>
                        <tr>
                            <th>(t("Name"))</th>
                            <th>(t("Type"))</th>
                            <th>(t("Value"))</th>
                            <th>(t("TTL"))</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        @for record in &zone.records {
                            <tr>
                                <td><code title=(&record.fqdn)>(&record.name)</code></td>
                                <td><span class="badge">(record.kind)</span></td>
                                <td class="record-value">
                                    @if let Some(priority) = record.priority {
                                        <span class="tile-detail">(priority.to_string())" "</span>
                                    }
                                    <code>(&record.value)</code>
                                </td>
                                <td class="tile-detail">(record.ttl.to_string())</td>
                                <td class="row-actions">
                                    @if zone.editable {
                                        <form method="post"
                                              action=(format!("{here}/records/{}/delete", record.id))>
                                            <button class="btn btn-ghost destructive btn-sm"
                                                    type="submit">(t("Remove"))</button>
                                        </form>
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
