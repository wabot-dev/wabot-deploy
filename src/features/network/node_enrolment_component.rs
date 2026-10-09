//! Minting a join token, spending one, and the terms each states.

use crate::infra::network;
use crate::infra::network::enrolment::Enrolment;
use crate::web::views::texts::language::t;
use hypertext::prelude::*;

/// The terms of a join, before it is a join.
///
/// Two lists and a decision. What the other node is **asking** of this
/// one, each item its own checkbox because each is refusable on its
/// own — a node may agree to serve somebody's names and never run their
/// containers — and what it **offers** in return, which is not a
/// checkbox because it is not this node's to accept or decline: it is
/// what the other node has already decided to allow.
///
/// Ticking nothing is a real answer and stays available. A node can
/// join to *use* another one and agree to do nothing for it, and a
/// screen that made the empty choice impossible would be asking for
/// consent it had already decided to have.
pub fn review_card<'a>(
    token: &'a str,
    terms: &'a network::token::JoinToken,
) -> impl Renderable + 'a {
    let requires = terms.requires();
    let offers = terms.offers();
    rsx! {
        <section class="stack">
            <p class="card-label">(t("Before you join"))</p>
            <form method="post" action="/network/join" class="card stack">
                <input type="hidden" name="token" value=(token)>
                <input type="hidden" name="agreed" value="1">

                <p class="tile-detail">
                    (format!("{} is asking to be able to:", terms.name))
                </p>
                @if requires.is_empty() {
                    <p class="tile-detail">(t("Nothing. It is handing this node an address on its \
                         overlay and asking for nothing back."))</p>
                } @else {
                    @for capability in &requires {
                        <label class="capability">
                            <input type="checkbox" name=(format!("accept-{}", capability.name()))
                                   value="1" checked>
                            <span>
                                <strong>(crate::web::views::components::node_capabilities::asking(*capability))</strong>
                                <span class="tile-detail">(crate::web::views::components::node_capabilities::why(*capability))</span>
                            </span>
                        </label>
                    }
                    <p class="field-hint">(t("Untick anything you would rather not agree to. You can \
                         revoke any of it later from this page, and this node \
                         keeps working either way."))</p>
                }

                <p class="tile-detail">(format!("In return, {} lets this node:", terms.name))</p>
                @if offers.is_empty() {
                    <p class="tile-detail">(t("Nothing. This node will take its instructions and give \
                         it none."))</p>
                } @else {
                    <ul class="notes">
                        @for capability in &offers {
                            <li>(crate::web::views::components::node_capabilities::offering(*capability))</li>
                        }
                    </ul>
                }

                <div class="actions">
                    <button type="submit">(t("Join"))</button>
                    <a class="btn btn-ghost" href="/network">(t("Cancel"))</a>
                </div>
            </form>
        </section>
    }
}

/// Mint a token for a node that is not here yet — or say why not.
///
/// The refusal is a card rather than a hidden form: "this node cannot
/// do that" is an answer, and a section that vanishes leaves somebody
/// looking for the thing they read about.
pub fn enrol_card(may_enrol: bool) -> impl Renderable {
    rsx! {
        <section class="stack">
            <p class="card-label">(t("Invite a node"))</p>
            @if may_enrol {
                <form method="post" action="/network/enrol" class="card stack">
                    // Not "what to call it". A node's name is its own —
                    // it says what it is on the join callback, and that
                    // is what the nodes list shows. Asking for one here
                    // produced a second name for one machine, differing
                    // by which table you read: the same fault the
                    // databases had, one floor up.
                    <label for="name">(t("A note, if you want one"))</label>
                    <input id="name" name="name" type="text" autocomplete="off"
                           placeholder=(t("the box in the rack by the window"))>
                    <p class="field-hint">(t("Only to tell one unspent token from another. The node \
                         names itself when it joins, and that is the name this \
                         console shows."))</p>

                    // Both lists are drawn from `Capability::ALL`, not
                    // written out. The *handler* has always read them by
                    // name — `require-{name}` — and the form had two
                    // boxes typed by hand, so adding a third capability
                    // left it unaskable: the code that granted it was
                    // complete and the only screen that could request it
                    // did not know it existed.
                    <p class="tile-detail">(t("Ask that node to let this one:"))</p>
                    @for capability in crate::domain::values::capability::Capability::ALL {
                        <label class="capability">
                            <input type="checkbox" name=(format!("require-{}", capability.name()))
                                   value="1" checked>
                            // `offering`, not `asking`: these two are
                            // written from the granting node's side, and
                            // this list is what *this* node wants to be
                            // allowed to do over there. Swapping them
                            // reads plausibly and says the opposite.
                            <span>(t(crate::web::views::components::node_capabilities::offering(capability)))</span>
                        </label>
                    }

                    <p class="tile-detail">(t("And offer it, in return:"))</p>
                    @for capability in crate::domain::values::capability::Capability::ALL {
                        <label class="capability">
                            <input type="checkbox" name=(format!("offer-{}", capability.name()))
                                   value="1">
                            <span>(t(crate::web::views::components::node_capabilities::asking(capability)))</span>
                        </label>
                    }
                    <p class="field-hint">(t("Both lists travel inside the token and are shown on the \
                         other machine before it is spent. Whoever holds it \
                         accepts or refuses each one — so what is asked for here \
                         is a request, not a setting."))</p>

                    <p class="field-hint">(t("This mints a token carrying this node's address, its overlay \
                         key and an address on the overlay for the new node. Joining \
                         with it there records this node as an authority — which that \
                         node can revoke at any time — and tells this one it arrived."))</p>
                    // The failure that will actually happen, said beside
                    // the button rather than discovered on the far
                    // machine: joining calls back over this hostname,
                    // and a node serving its own authority's certificate
                    // is one nothing else trusts yet.
                    <p class="field-hint">(t("The other machine has to trust the certificate this console \
                         is served on. Until this node has a public one, joining will \
                         refuse rather than send its token to whatever answered."))</p>
                    <div class="actions">
                        <button type="submit">(t("Create join token"))</button>
                    </div>
                </form>
            } @else {
                <section class="card stack">
                    <p>(t("This node has no address another one could call back on, so it \
                         cannot enrol anybody yet."))</p>
                    <p class="field-hint">(t("Set a domain on this node's own page. A joining node reaches it \
                         over the same hostname and certificate this console is served \
                         on, and that certificate has to be one the other machine \
                         already trusts."))</p>
                </section>
            }
        </section>
    }
}

/// Paste a token from somewhere else, and what became of the ones this
/// node minted.
///
/// One section, because they are two halves of the same question — the
/// tokens this node handed out, and the one it was handed.
/// The tokens this node has minted and nobody has spent.
///
/// It was inside `join_card`, which put "what I have offered other nodes"
/// under the heading "join a network" — the opposite direction. It belongs
/// beside the form that mints them.
pub fn minted_card<'a>(
    enrolments: &'a [Enrolment],
    nodes: &'a [network::Node],
    now: i64,
) -> impl Renderable + 'a {
    rsx! {
        @if !enrolments.is_empty() {
            <section class="stack">
                <p class="card-label">(t("Tokens this node minted"))</p>
                <table>
                    <thead>
                        <tr>
                            <th>(t("For"))</th><th>(t("Address"))</th>
                            <th>(t("State"))</th><th></th>
                        </tr>
                    </thead>
                    <tbody>
                        @for enrolment in enrolments {
                            (enrolment_row(enrolment, nodes, now))
                        }
                    </tbody>
                </table>
            </section>
        }
    }
}

pub fn join_card(joined: Option<&str>) -> impl Renderable + '_ {
    rsx! {
        <section class="stack">
            <p class="card-label">(t("Join a network"))</p>
            @if let Some(name) = joined {
                <section class="card stack">
                    <p>(t("This node now takes instructions from "))(name)(".")</p>
                    <p class="field-hint">(t("Only what it was granted, which is listed beside it below. \
                         Revoking any of it takes effect here and immediately."))</p>
                </section>
            }
            <form method="post" action="/network/join" class="card stack">
                <label for="token">(t("Join token"))</label>
                <input id="token" name="token" type="text" class="mono"
                       placeholder="wdj1.…" autocomplete="off" required>
                <p class="field-hint">(t("From the network page of the node you are joining: invite a node \
                     there, and it shows one token, once. Pasting it here does \
                     what `wabot-deploy join` does in a terminal — this node records \
                     that one as an authority and tells it so. The same token can be \
                     pasted again if something goes wrong part-way."))</p>
                <div class="actions">
                    <button type="submit">(t("Join"))</button>
                </div>
            </form>
        </section>
    }
}

/// One join token, and what became of it.
///
/// `nodes` is every node this one knows, so a spent token can be shown
/// under the name of the machine that spent it. The note the operator
/// left is a stand-in until then, and it stops being shown the moment
/// there is something truer: a machine has one name, and it is the one
/// it gives itself.
pub fn enrolment_row<'a>(
    enrolment: &'a Enrolment,
    nodes: &'a [network::Node],
    now: i64,
) -> impl Renderable + 'a {
    let spent = enrolment.spent();
    let called = enrolment
        .used_by
        .as_deref()
        .and_then(|id| nodes.iter().find(|node| node.id == id))
        .map(|node| node.name.as_str())
        .unwrap_or(match enrolment.name.is_empty() {
            // Nothing to say rather than an empty cell: the address
            // beside it is what identifies an unspent token, and it is
            // already there.
            true => "—",
            false => enrolment.name.as_str(),
        });
    rsx! {
        <tr>
            <td>(called)</td>
            <td class="mono">(&enrolment.overlay_ip)</td>
            <td>
                // "Is it still worth carrying to a machine" first,
                // because it is the question somebody opens this table
                // to answer.
                @if enrolment.live(now) {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>(t("Waiting"))
                    </span>
                } @else if spent {
                    <span class="badge">(t("Used"))</span>
                } @else {
                    <span class="badge badge-warning">(t("Expired"))</span>
                }
            </td>
            <td>
                // A used token is history, and withdrawing one would
                // free an address a node is already answering to.
                @if !spent {
                    <form method="post"
                          action=(format!("/network/enrolments/{}/withdraw", enrolment.id))>
                        <button class="btn btn-ghost destructive btn-sm" type="submit">
                            (t("Withdraw"))
                        </button>
                    </form>
                }
            </td>
        </tr>
    }
}

/// One authority, named by the node it belongs to when that node is
/// known here — an id is not something anybody recognises.
pub fn authority_row<'a>(
    authority: &'a network::Authority,
    nodes: &'a [network::Node],
    held: &'a [crate::domain::values::capability::Capability],
) -> impl Renderable + 'a {
    let name = nodes
        .iter()
        .find(|node| node.id == authority.node_id)
        .map(|node| node.name.as_str())
        .unwrap_or(authority.node_id.as_str());

    rsx! {
        <tr>
            <td>
                (name)
                <span class="tile-detail">(&authority.node_id)</span>
            </td>
            <td>
                @if held.is_empty() {
                    <span class="tile-detail">(t("nothing"))</span>
                } @else {
                    @for capability in held {
                        <span class="badge">(capability.name())</span>
                    }
                }
            </td>
            <td>
                @if authority.live() {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>(t("Allowed"))
                    </span>
                } @else {
                    <span class="badge">(t("Revoked"))</span>
                }
            </td>
            <td>
                @if authority.live() {
                    <form method="post"
                          action=(format!("/network/revoke/{}", authority.node_id))>
                        <button class="btn btn-ghost destructive btn-sm" type="submit">
                            (t("Revoke"))
                        </button>
                    </form>
                }
            </td>
        </tr>
    }
}
