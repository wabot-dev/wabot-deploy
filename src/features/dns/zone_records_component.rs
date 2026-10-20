use hypertext::prelude::*;

use crate::domain::values::record_data::RecordKind;
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

            @if zone.editable {
                <form method="post" action=(format!("{here}/records")) class="stack">
                    <input type="hidden" name="zone" value=(&zone.name)>
                    <label for="record-name">(t("Name"))</label>
                    <input id="record-name" name="name" type="text" autocomplete="off" placeholder="@">
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
                    </div>
                </form>
            } @else {
                <p class="field-hint">(t("Sent by another node: it is changed there, and this node only answers for it."))</p>
            }
        </section>
    }
}
