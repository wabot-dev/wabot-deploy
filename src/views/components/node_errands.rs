//! What this node has been asked to do, and what it asked of others.

use crate::network;
use crate::views::texts::language::t;
use hypertext::prelude::*;

/// One instruction, and what became of it.
///
/// A failure is an *answer*, and it says what it was — the state worth
/// worrying about is the one that never came back, which is why "asked
/// for and never settled" is its own word rather than an absence.
/// What this node has asked that one to do, and how far it has got.
///
/// Its own island host because the errand table is on a *joined* node's
/// page, and the live cards are on this node's own — two pages, so two
/// hosts, both talking to the same stream.
pub fn errands_card<'a>(
    node_id: &'a str,
    orders: &'a [network::errand::Record],
) -> impl Renderable + 'a {
    let inner = rsx! {
        @if !orders.is_empty() {
            <table>
                <thead>
                    <tr>
                        <th>(t("Asked"))</th><th>(t("State"))</th>
                    </tr>
                </thead>
                <tbody>
                    @for order in orders {
                        (errand_row(order))
                    }
                </tbody>
            </table>
        }
    }
    .render()
    .into_inner();

    wabot::ui::hypertext::island(
        "node-live",
        &serde_json::json!({ "node": node_id }),
        hypertext::Raw::dangerously_create(&inner),
    )
}

pub fn errand_row(order: &network::errand::Record) -> impl Renderable + '_ {
    rsx! {
        <tr>
            <td>
                (order.kind.as_str())
                // A second line, for the reason written on the backup
                // row above: a `.tile-detail` span renders flush against
                // what precedes it, so this read as `hostnd-BuhSasl…`.
                <p class="tile-detail">(&order.id)</p>
            </td>
            <td data-errand=(&order.id)>
                @if let Some(reason) = &order.error {
                    <span class="badge badge-danger">
                        <span class="dot dot-danger"></span>(t("Refused"))
                    </span>
                    <p class="failure">(reason)</p>
                } @else if order.done() {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>(t("Done"))
                    </span>
                } @else if order.taken_at.is_some() {
                    <span class="badge badge-info">
                        <span class="dot dot-info dot-pulse"></span>(t("Collected"))
                    </span>
                } @else {
                    <span class="badge badge-info">
                        <span class="dot dot-info"></span>(t("Waiting to be collected"))
                    </span>
                }
            </td>
        </tr>
    }
}
