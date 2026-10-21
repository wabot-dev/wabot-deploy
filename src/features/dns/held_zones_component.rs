use hypertext::prelude::*;

use crate::features::dns::dns_response::DnsResponse;
use crate::features::dns::texts::t;

pub fn held_card<'a>(dns: &'a DnsResponse) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("Zones in no project"))</p>
            @if dns.held.is_empty() {
                <p class="tile-detail">(t("None. A zone lives in the project whose people manage it; the ones listed here are those another node sent for this one to answer for."))</p>
            } @else {
                <table>
                    <thead>
                        <tr>
                            <th>(t("Domain"))</th>
                            <th>(t("Whose"))</th>
                            <th>(t("Records"))</th>
                            <th>(t("Here"))</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        @for zone in &dns.held {
                            <tr>
                                <td><code>(&zone.name)</code></td>
                                <td class="tile-detail">
                                    @if let Some(node) = &zone.sent_by {
                                        (t("Sent by")) " " (node)
                                    } @else {
                                        (t("In no project"))
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
                                <td class="row-actions">
                                    <form method="post" action=(format!("/node/dns/{}/delete", zone.name))>
                                        <button class="btn btn-ghost destructive btn-sm" type="submit">
                                            @if zone.sent_by.is_some() { (t("Stop answering")) } @else { (t("Delete")) }
                                        </button>
                                    </form>
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
                <p class="field-hint">(t("A zone another node sent stops being answered here and its records are forgotten. That node is not told, and sends it again the next time it changes — to stop that, withdraw what this node lets it do. One in no project is deleted with its records."))</p>
            }
        </section>
    }
}
