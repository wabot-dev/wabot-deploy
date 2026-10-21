use hypertext::prelude::*;

use crate::domain::entities::redirect::Redirect;
use crate::features::services::texts::t;
use crate::shell::components::node_certificate::CertificateCells;

/// `add` is where a redirect is made, or `None` when there is nothing to
/// redirect to — a service with no hostname, or one sent from elsewhere.
pub fn redirects_card<'a>(
    redirects: &'a [(Redirect, CertificateCells)],
    add: Option<&'a str>,
    remove: &'a str,
) -> impl Renderable + 'a {
    rsx! {
        <section class="stack" id="redirects">
            <div class="split">
                <p class="card-label">(t("Redirects"))</p>
                @if let Some(add) = add {
                    <a class="btn btn-secondary" href=(add)>(t("Add redirect"))</a>
                }
            </div>
            @if redirects.is_empty() {
                <p class="tile-detail">
                    @if add.is_some() {
                        (t("None. A redirect sends another name — www.example.com, a domain you used to have — to one of this service's hostnames, path and query kept."))
                    } @else {
                        (t("None. A redirect sends another name to one of this service's hostnames, and it has none yet: expose a port over HTTPS first."))
                    }
                </p>
            } @else {
                <table>
                    <thead>
                        <tr>
                            <th>(t("From"))</th>
                            <th>(t("To"))</th>
                            <th>(t("Certificate"))</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        @for (redirect, cells) in redirects {
                            <tr>
                                <td class="mono">(&redirect.hostname)</td>
                                <td class="mono">(&redirect.target)</td>
                                <td>
                                    <span class=(cells.badge)>
                                        <span class=(cells.dot)></span>
                                        <span>(cells.word)</span>
                                    </span>
                                </td>
                                <td>
                                    <form method="post"
                                          action=(format!("{remove}/{}/delete", redirect.hostname))>
                                        <button class="btn btn-ghost destructive btn-sm" type="submit">
                                            (t("Remove"))
                                        </button>
                                    </form>
                                </td>
                            </tr>
                            @if !cells.failure.is_empty() {
                                <tr class="port-detail">
                                    <td colspan="4">
                                        <p class="failure">(&cells.failure)</p>
                                    </td>
                                </tr>
                            }
                        }
                    </tbody>
                </table>
            }
        </section>
    }
}
