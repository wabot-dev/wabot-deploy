//! What this node is willing to do, and what it has agreed to do for
//! another one.

use crate::network;
use crate::views::texts::language::t;
use hypertext::prelude::*;

/// What this node has agreed that node may ask of it.
///
/// **The local decision, and the only one this page can change.** The
/// other direction — what that node lets *this* one ask of it — is on its
/// row as `allows`, learned from its reports, and reading one for the
/// other is the trap phase 8 walked into: they answer opposite questions.
/// So both are here, and only one of them is a form.
pub fn grants_card<'a>(
    node: &'a network::Node,
    granted: &'a [(network::capability::Capability, bool)],
) -> impl Renderable + 'a {
    rsx! {
        <section class="stack">
            <p class="card-label">(t("What this node does for it"))</p>
            <form method="post" action=(format!("/network/{}/grants", node.id)) class="card stack">
                @for (capability, on) in granted {
                    <label class="capability">
                        @if *on {
                            <input type="checkbox" name=(capability.name()) value="1" checked>
                        } @else {
                            <input type="checkbox" name=(capability.name()) value="1">
                        }
                        <span>
                            <strong>(t(asking(*capability)))</strong>
                            <span class="tile-detail">(t(why(*capability)))</span>
                        </span>
                    </label>
                }
                <p class="field-hint">(t("A grant is this node's own decision and lives on this \
                     machine. Withdrawing one asks nobody's permission — consent \
                     is for taking work on, not for putting it down."))</p>
                <div class="actions">
                    <button type="submit">(t("Save"))</button>
                </div>
            </form>

            <div class="card stack">
                <p class="card-label">(t("What it lets this node ask of it"))</p>
                @if node.allows.is_empty() {
                    <p class="tile-detail">(t("Nothing. Anything asked of it would be refused, so this \
                         node does not offer it as somewhere to place a replica, \
                         serve a name or keep a copy."))</p>
                } @else {
                    <ul class="notes">
                        @for capability in &node.allows {
                            <li>(t(offering(*capability)))</li>
                        }
                    </ul>
                }
                <p class="field-hint">(t("Learned from what that node reports, because the decision is \
                     a row on that machine. Changing it is done from its own \
                     console."))</p>
            </div>
        </section>
    }
}

/// What holding this capability lets the other node do here.
pub fn asking(capability: network::capability::Capability) -> &'static str {
    match capability {
        network::capability::Capability::Host => "Run its containers on this node",
        network::capability::Capability::Edge => "Have this node answer for its hostnames",
        network::capability::Capability::Store => "Keep a copy of its data on this node",
        network::capability::Capability::Manage => "Keep this node up to date, and restart it",
    }
}

pub fn why(capability: network::capability::Capability) -> &'static str {
    match capability {
        network::capability::Capability::Host => {
            "It can place replicas of its services here and pull the images \
             for them. It cannot touch anything else on this node."
        }
        network::capability::Capability::Edge => {
            "It can ask this node to terminate TLS for one of its names and \
             proxy to wherever that service runs."
        }
        network::capability::Capability::Store => {
            "It can place a read-only copy of one of its databases here, and \
             that copy is every row of it — on this node's disk, taking this \
             node's space. Running somebody's container is not the same \
             favour, which is why this is asked for separately."
        }
        network::capability::Capability::Manage => {
            "It can install a release of this project here, restart the service \
             and ask for a backup. It creates nothing, reads no data and touches \
             no account — but it decides which binary this node runs."
        }
    }
}

pub fn offering(capability: network::capability::Capability) -> &'static str {
    match capability {
        network::capability::Capability::Host => "Run this node's containers over there",
        network::capability::Capability::Edge => "Answer for this node's hostnames",
        network::capability::Capability::Store => "Keep a copy of this node's data over there",
        network::capability::Capability::Manage => "Update and restart that node from here",
    }
}

/// What the switch for one capability says on this node's own page.
///
/// Separate from [`asking`], which is the same capability seen from the
/// other side of a join: there it is "what that node may do here", and
/// here it is "what this node does at all". Exhaustive, so a capability
/// added later cannot reach the form without words — which is how `store`
/// came to have no checkbox on a form that wrote its setting anyway.
pub fn doing(capability: network::capability::Capability) -> &'static str {
    match capability {
        network::capability::Capability::Host => "Run containers",
        network::capability::Capability::Edge => "Answer for hostnames",
        network::capability::Capability::Store => "Keep copies of other nodes' data",
        network::capability::Capability::Manage => "Be kept up to date from elsewhere",
    }
}

/// And what turning it off costs. `reachable` is only read by `Edge`,
/// which is the one capability an address decides.
pub fn doing_why(capability: network::capability::Capability, reachable: bool) -> &'static str {
    match capability {
        network::capability::Capability::Host => {
            "Replicas can be placed here — its own services included. A \
             small node can own projects and run none of them."
        }
        network::capability::Capability::Edge if reachable => {
            "It terminates TLS for names, its own and other nodes'. Turning \
             this off makes it a private node — that is all private has ever \
             meant."
        }
        network::capability::Capability::Edge => {
            "Needs a domain that resolves here. Without an address the world \
             can dial, this node is private whatever it would prefer."
        }
        network::capability::Capability::Store => {
            "A standby of somebody's database lives on this node's disk and \
             grows with it. Running a container is not the same favour."
        }
        network::capability::Capability::Manage => {
            "A node that has been granted this can install a release here, \
             restart the service and ask for a backup. Turning it off \
             withdraws that from every node at once."
        }
    }
}

/// What this node does at all, and whether it originates any of it.
///
/// **Generated from `Capability::ALL`, not written out.** The first
/// version drew two checkboxes while the handler wrote a setting for
/// every capability in the enum, so saving this form turned `store` off
/// in silence: the node stopped being offered as somewhere a standby
/// could go, and refused the errands for the ones it already held. A
/// capability with no checkbox has to be impossible rather than
/// noticed, which is what a loop and an exhaustive `match` make it.
pub fn capabilities_card(
    provides: &[(network::capability::Capability, bool)],
    reachable: bool,
    owns: bool,
    owned: usize,
) -> impl Renderable + '_ {
    rsx! {
        <section class="stack">
            <p class="card-label">(t("What this node does"))</p>
            <form method="post" action="/node/capabilities" class="card stack">
                @for (capability, on) in provides {
                    <label class="capability">
                        // A capability that needs an address, on a node
                        // that has none, is a switch that could be turned
                        // on to no effect.
                        @if capability.needs_endpoint() && !reachable {
                            <input type="checkbox" name=(capability.name()) value="1" disabled>
                        } @else if *on {
                            <input type="checkbox" name=(capability.name()) value="1" checked>
                        } @else {
                            <input type="checkbox" name=(capability.name()) value="1">
                        }
                        <span>
                            <strong>(t(doing(*capability)))</strong>
                            <span class="tile-detail">(t(doing_why(*capability, reachable)))</span>
                        </span>
                    </label>
                }

                // Not one of them, and deliberately below them with its
                // own words: every capability above is something another
                // node asks of this one, and this is what this node is
                // for. It has no grant and appears in no token.
                <label class="capability">
                    @if owns {
                        <input type="checkbox" name="own" value="1" checked>
                    } @else {
                        <input type="checkbox" name="own" value="1">
                    }
                    <span>
                        <strong>(t("Own projects and services"))</strong>
                        <span class="tile-detail">(t("Off, this node originates nothing: no projects \
                             of its own, no services, no names. What arrived \
                             on an errand goes on running — it was never this \
                             node's."))</span>
                        @if owns && owned > 0 {
                            <span class="tile-detail">(t("This node owns "))(owned.to_string())
                                (t(" of its own projects, and has to let go of them before this \
                                     can be turned off."))</span>
                        }
                    </span>
                </label>

                // The write-ahead log used to be here, as a third thing
                // the node either does or does not do. It is on the
                // Backup tab now, beside the schedule and the window it
                // is bounded by: point-in-time recovery is the *reach* of
                // a backup — no base copy, nothing to replay the log onto
                // — and a switch two tabs away from the thing it depends
                // on is a switch somebody turns on and gains nothing by.

                <div class="actions">
                    <button type="submit">(t("Save"))</button>
                </div>
            </form>
        </section>
    }
}
