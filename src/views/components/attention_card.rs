//! What the console shows when something needs somebody.
//!
//! One card, on whichever page the concern belongs to. A failure that
//! only reached the journal is a failure nobody sees.

use hypertext::prelude::*;

use crate::console::attention::Concern;
use crate::views::texts::language::t;

/// The card, or nothing at all.
///
/// **Nothing when there is nothing**, which is the property that makes
/// the rest of it worth anything: a panel that is always on the page
/// becomes part of the wallpaper, and the day it has something to say
/// it says it in the same place it says "all well". An operator should
/// be able to tell at a glance, from whether the card is there.
///
/// Not a stream. These are read when the page is drawn, and a page an
/// operator is looking at is one they can reload — where a badge that
/// updates itself would mean a list that reorders under a cursor
/// half-way to clicking something.
pub fn card(concerns: &[Concern]) -> impl Renderable + '_ {
    rsx! {
        @if !concerns.is_empty() {
            <section class="stack">
                <p class="card-label">(t("Needs you"))</p>
                <div class="card">
                    <table>
                        <tbody>
                            @for concern in concerns {
                                <tr>
                                    <td>
                                        <span class=(concern.weight.badge())>
                                            <span class=(concern.weight.dot())></span>
                                            (crate::views::texts::language::word(&concern.what))
                                        </span>
                                    </td>
                                    // Somebody's own string, or the
                                    // machine's: never translated, like
                                    // every hostname and slug here.
                                    <td class="mono">(&concern.which)</td>
                                    <td class="tile-detail">(&concern.why)</td>
                                    <td>
                                        <a class="btn btn-ghost btn-sm"
                                           href=(&concern.go)>(t("Look"))</a>
                                    </td>
                                </tr>
                            }
                        </tbody>
                    </table>
                </div>
            </section>
        }
    }
}
