use hypertext::prelude::*;

use crate::features::dns::texts::t;
use crate::features::dns::zone_response::NameServer;

/// What to type at the registrar. The addresses are what each name leads
/// to now, so a name that leads nowhere is visible before it is typed.
pub fn registrar_card<'a>(name_servers: &'a [NameServer]) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("At your domain's registrar"))</p>
            @if name_servers.is_empty() {
                <p class="tile-detail">(t("No node answers for this zone yet, so there is nothing to set there."))</p>
            } @else {
                <p class="tile-detail">(t("Replace the name servers your registrar lists for this domain with these, and only these:"))</p>
                <table>
                    <tbody>
                        @for server in name_servers {
                            <tr>
                                <td><code>(&server.name)</code></td>
                                <td class="tile-detail">
                                    @if server.addresses.is_empty() {
                                        (t("does not resolve yet"))
                                    } @else {
                                        (server.addresses.join(", "))
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
                <p class="field-hint">(t("Resolvers that remember the old ones can take up to two days to ask these instead."))</p>
            }
        </section>
    }
}
