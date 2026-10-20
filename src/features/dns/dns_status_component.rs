use hypertext::prelude::*;

use crate::features::dns::texts::t;
use crate::infra::dns::server::Listening;

pub fn status_card<'a>(
    provides: bool,
    addressed: bool,
    has_zones: bool,
    listening: &'a [Listening],
) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("This node as a name server"))</p>
            @if !addressed {
                <p class="tile-detail">
                    <span class="badge"><span class="dot dot-info"></span>(t("Off"))</span>
                    " "
                    (t("It needs a domain that resolves here first: a name server the world cannot reach is one nobody can ask."))
                </p>
            } @else if !provides {
                <p class="tile-detail">
                    <span class="badge"><span class="dot dot-info"></span>(t("Off"))</span>
                    " "
                    (t("Zones can be prepared, and none is answered until Answer DNS is turned on for this node."))
                </p>
                <p><a href="/node">(t("This node's settings"))</a></p>
            } @else if !has_zones || listening.is_empty() {
                <p class="tile-detail">
                    <span class="badge badge-info"><span class="dot dot-info"></span>(t("On"))</span>
                    " "
                    (t("Port 53 opens with the first zone this node answers for, and closes with the last."))
                </p>
            } @else {
                <table>
                    <tbody>
                        @for place in listening {
                            <tr>
                                <td><code>(place.address.to_string())</code></td>
                                <td>
                                    @if let Some(reason) = &place.refused {
                                        <span class="badge badge-danger"><span class="dot dot-danger"></span>(t("Not answering"))</span>
                                        " "
                                        <span class="tile-detail">(reason)</span>
                                    } @else {
                                        <span class="badge badge-success"><span class="dot dot-success"></span>(t("Answering"))</span>
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
