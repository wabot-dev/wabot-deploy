use hypertext::prelude::*;

use crate::features::dns::texts::t;
use crate::features::dns::zones_response::ZonesResponse;

/// The names this node is listed under in every zone it answers for, and
/// whether each really leads here — a registrar accepts a name that leads
/// nowhere, and the domain then answers nothing.
pub fn names_card<'a>(zones: &'a ZonesResponse) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("The names this node answers as"))</p>
            @if zones.answers_as.is_empty() {
                <p class="tile-detail">(t("None yet: this node has no domain, and no name has been set for it."))</p>
            } @else {
                <table>
                    <tbody>
                        @for name in &zones.answers_as {
                            <tr>
                                <td><code>(&name.name)</code></td>
                                <td>
                                    @if name.addresses.is_empty() {
                                        <span class="badge badge-warning"><span class="dot dot-warning"></span>(t("Does not resolve yet"))</span>
                                    } @else if name.here {
                                        <span class="badge badge-success"><span class="dot dot-success"></span>(t("Points here"))</span>
                                        " "
                                        <span class="tile-detail">(name.addresses.join(", "))</span>
                                    } @else {
                                        <span class="badge badge-danger"><span class="dot dot-danger"></span>(t("Points elsewhere"))</span>
                                        " "
                                        <span class="tile-detail">(name.addresses.join(", "))</span>
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            }
            <form method="post" action="/dns/names" class="stack">
                <label for="name-servers">(t("Names"))</label>
                <textarea id="name-servers" name="names" rows="3" autocomplete="off" class="mono"
                          placeholder="ns1.example.com">(zones.declared.join("\n"))</textarea>
                <p class="field-hint">(t("One per line, like ns1.example.com and ns2.example.com. Each needs an A record pointing at this node, wherever that domain's DNS is kept — and when that is this node, a glue record at its registrar too. Left empty, the node answers as its own domain."))</p>
                <p class="field-hint">(t("Two names for one node satisfy a registrar that asks for two, and they are still one machine: when it stops, both stop answering."))</p>
                <div class="actions">
                    <button type="submit">(t("Save names"))</button>
                </div>
            </form>
        </section>
    }
}
