//! Where a service's copies run, and who answers for its names.

use crate::views::texts::language::t;
use hypertext::prelude::*;

/// What a replica's node is called, for a table that reads rather than
/// selects.
pub fn node_name<'a>(
    replica: &'a crate::entities::replica::Replica,
    nodes: &'a [crate::network::Node],
) -> String {
    match &replica.node_id {
        Some(id) => nodes
            .iter()
            .find(|node| &node.id == id)
            .map(|node| node.name.clone())
            .unwrap_or_else(|| id.clone()),
        None => nodes
            .iter()
            .find(|node| node.is_self)
            .map(|node| node.name.clone())
            .unwrap_or_else(|| t("this node").to_string()),
    }
}

/// Where every copy of this service runs, and how to change it.
///
/// One row per replica, each naming the node it is on. The first is
/// this node by default and there is nothing special about it — "stays
/// here" is a placement like any other, which is the whole point of
/// administering a service from where it was created rather than from
/// the machine it happens to be on.
pub fn placement_card<'a>(
    project: &'a crate::entities::project::Project,
    service: &'a crate::entities::service::Service,
    placements: &'a [crate::entities::replica::Replica],
    nodes: &'a [crate::network::Node],
    // The copies out of the rotation, by replica id — see `Running`.
    silent: &'a [String],
    // The nodes with an instruction about this service still waiting to
    // be collected, so a copy that has been told reads apart from one
    // that has not.
    queued: &'a [String],
) -> impl Renderable + 'a {
    let action = format!(
        "/projects/{}/services/{}/placement",
        project.slug, service.slug
    );
    rsx! {
        <section class="stack" id="placement">
            <div class="split">
                <p class="card-label">(t("Where this runs"))</p>
                <span class="who">(format!("{} replica(s)", placements.len()))</span>
            </div>

            <form method="post" action=(&action) class="card stack">
                <table>
                    <thead>
                        <tr><th>(t("Replica"))</th><th>(t("Node"))</th><th>(t("State"))</th></tr>
                    </thead>
                    <tbody>
                        @for replica in placements {
                            <tr>
                                <td class="mono">("#")(replica.slot)</td>
                                <td>
                                    <select name=(format!("slot-{}", replica.slot))>
                                        @for node in nodes {
                                            (node_option(node, replica))
                                        }
                                    </select>
                                </td>
                                <td data-replica=(&replica.id)>
                                    (placement_state(
                                        replica,
                                        service.desired_state == crate::entities::service::DesiredState::Stopped,
                                        replica
                                            .node_id
                                            .as_ref()
                                            .is_some_and(|node| queued.iter().any(|q| q == node)),
                                        silent.contains(&replica.id),
                                    ))
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>

                <div class="placement-count">
                    <div>
                        <label for="replicas">(t("How many"))</label>
                        <input id="replicas" name="replicas" type="number" autocomplete="off" min="1" max="16"
                               value=(placements.len().to_string())>
                    </div>
                    <div>
                        <label for="new-on">(t("New ones on"))</label>
                        <select id="new-on" name="new-on">
                            <option value=(crate::console::service_rest_controller::ANYWHERE)>(t("Wherever there is room"))</option>
                            @for node in nodes {
                                @if node.is_self {
                                    <option value="" selected>(&node.name)</option>
                                } @else {
                                    <option value=(&node.id)>(&node.name)</option>
                                }
                            }
                        </select>
                    </div>
                </div>
                <p class="field-hint">(t("A new copy is created on the node you pick, rather than here and \
                     moved after — which would start a container on this machine and \
                     stop it again for nothing. Removing takes the ones already thrown \
                     out first, then the highest-numbered; the node running one is told \
                     to stop it."))</p>
                <p class="field-hint">(t("Wherever there is room spreads new copies across nodes that have \
                     agreed to run this and have the memory for it, emptiest first. It \
                     writes an ordinary row saying why, and never moves one you placed \
                     yourself."))</p>
                <div class="actions">
                    <button type="submit">(t("Save placement"))</button>
                </div>
            </form>
        </section>
    }
}

/// One node in a replica's selector.
///
/// This node is the **empty** value, because "here" is the absence of a
/// placement elsewhere — the same shape the row has, so the form and
/// the column agree without anything translating between them.
pub fn node_option<'a>(
    node: &'a crate::network::Node,
    replica: &'a crate::entities::replica::Replica,
) -> impl Renderable + 'a {
    let value = match node.is_self {
        true => String::new(),
        false => node.id.clone(),
    };
    let chosen = match node.is_self {
        true => replica.is_here(),
        false => replica.node_id.as_deref() == Some(node.id.as_str()),
    };
    rsx! {
        @if chosen {
            <option value=(value) selected>(&node.name)</option>
        } @else {
            <option value=(value)>(&node.name)</option>
        }
    }
}

/// Which public nodes answer for one of this service's names.
///
/// Only the owner sees it: an edge is part of administering the service,
/// and the whole point of the origin rule is that this is decided in one
/// place. A node that merely runs a replica has nothing to choose here.
///
/// Every choice is a checkbox rather than a single selector because a
/// name can be served from several nodes at once — that is what makes it
/// survive one of them going away.
///
/// The name is the heading and the nodes are a column under it. It was
/// one flex line — a label, the boxes and a button, all in a row — which
/// is what fitted inside the cell it used to live in and reads as one
/// run-on sentence anywhere else.
/// The most recent instruction to one node about one name.
///
/// The same filter `edge_cells` uses, so the two cannot disagree about
/// which errand is the current one.
pub fn latest_for<'a>(
    orders: &'a [crate::network::errand::Record],
    hostname: &str,
    node_id: &str,
) -> Option<&'a crate::network::errand::Record> {
    orders
        .iter()
        .filter(|order| order.node_id == node_id)
        .filter(|order| order.kind == crate::network::errand::Kind::Edge)
        .filter(|order| {
            order
                .payload
                .get("hostname")
                .and_then(|value| value.as_str())
                == Some(hostname)
        })
        .max_by_key(|order| order.created_at)
}

pub fn served_by_form<'a>(
    project: &'a crate::entities::project::Project,
    service: &'a crate::entities::service::Service,
    hostname: &'a str,
    public: &'a [crate::network::Node],
    serving: &'a [(String, String)],
    // Every instruction this node has queued, so the badge below can be
    // computed here rather than written optimistically and corrected by
    // a stream that may not reach this page.
    orders: &'a [crate::network::errand::Record],
) -> impl Renderable + 'a {
    let chosen: Vec<&str> = serving
        .iter()
        .filter(|(name, _)| name == hostname)
        .map(|(_, node)| node.as_str())
        .collect();
    rsx! {
        <form method="post" class="stack-sm"
              action=(format!(
                  "/projects/{}/services/{}/edges",
                  project.slug, service.slug
              ))>
            <input type="hidden" name="hostname" value=(hostname)>
            // The name this form is about, and it has to be here: two
            // ports with two hostnames render two of these, and without
            // it they are the same list of nodes twice. It read as a
            // label — "also served by" — only while the address was in
            // the cell next door.
            <p class="mono">(hostname)</p>
            @for node in public {
                <label class="check">
                    @if chosen.contains(&node.id.as_str()) {
                        <input type="checkbox"
                               name=(format!("edge-{}", node.id)) value="1" checked>
                    } @else {
                        <input type="checkbox"
                               name=(format!("edge-{}", node.id)) value="1">
                    }
                    <span>(&node.name)</span>
                    // How far the instruction to that node has got.
                    // Ticking a box used to be the end of what this
                    // page said: an errand went out, a name was
                    // claimed, a certificate was ordered, and none of
                    // it came back here — so the only honest reading of
                    // a ticked box was "somebody asked for this once".
                    // **Computed, not assumed.** This wrote `Asked`
                    // whenever a box was ticked and left the stream to
                    // replace it — so a page the stream does not reach
                    // said `Asked` for ever, and Jorge's node said it
                    // about a name it was already serving with a
                    // certificate. Its errand table was empty: with
                    // nothing pending the answer was always "serving",
                    // and nothing worked that out.
                    //
                    // The same function the stream uses, so the first
                    // paint and every update after it read the same.
                    @let cell = crate::views::components::project_cells::edge_cell(latest_for(orders, hostname, &node.id));
                    <span class=(match chosen.contains(&node.id.as_str()) {
                              true => cell.badge,
                              false => "badge badge-info is-hidden",
                          })
                          data-edge=(format!("{}|{}", hostname, node.id))>
                        <span class=(cell.dot)></span>(crate::views::texts::language::word(&cell.word))
                    </span>
                </label>
            }
            <div class="actions">
                <button class="btn btn-secondary btn-sm" type="submit">(t("Save"))</button>
            </div>
        </form>
    }
}

/// What a replica is doing, rendered from the one decision.
///
/// The words and the classes come from `projects::replica_cell`, which the
/// stream sends as data — so the first paint and every update after it
/// cannot disagree. They did: a stopped service's remote copy read "not
/// running" on load and "waiting for that node" two seconds later, because
/// the decision had been made twice and only one copy was corrected.
pub fn placement_state<'a>(
    replica: &'a crate::entities::replica::Replica,
    stopped: bool,
    queued: bool,
    not_answering: bool,
) -> impl Renderable + 'a {
    let cell = crate::views::components::project_cells::replica_cell(
        replica,
        stopped,
        queued,
        not_answering,
    );
    rsx! {
        @if cell.dot.is_empty() {
            <span class=(cell.badge)>(crate::views::texts::language::word(&cell.word))</span>
        } @else {
            <span class=(cell.badge)>
                <span class=(cell.dot)></span>(crate::views::texts::language::word(&cell.word))
            </span>
        }
        // The line under it: an address while it is up, a reason while it
        // is not. `failure` is the class the stream writes a reason into,
        // and it is what the page has always used for one.
        // **Always rendered, even empty.** The stream can only write into
        // an element the server already put there — `console.js` looks the
        // line up and does nothing when it is missing — so a row that had
        // nothing to say when the page loaded could never gain a reason.
        // A deployment that failed a second later flipped the badge to
        // `Failed` live and dropped the *why* on the floor, and it took a
        // manual refresh to see it. Reported by Jorge, mistyping an image
        // name.
        //
        // `:empty` hides it, so an empty line costs no space. The class
        // carries the severity and the island rewrites it from the badge,
        // because a line that starts grey and becomes a failure has to
        // turn red with it.
        @if cell.badge.contains("danger") {
            <p class="failure detail-line" data-detail>(&cell.detail)</p>
        } @else {
            <p class="tile-detail detail-line" data-detail>(&cell.detail)</p>
        }
    }
}

/// A service this node did not create.
///
/// Named rather than hidden: somebody looking at it needs to know which
/// node to go and argue with, and a page that simply showed fewer
/// buttons would leave them wondering what they had done wrong.
pub fn from_elsewhere_card<'a>(
    project: &'a crate::entities::project::Project,
    service: &'a crate::entities::service::Service,
    mine: &'a [crate::entities::replica::Replica],
    evicted: bool,
) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("From another node"))</p>
            <p>(t("This service is administered from the node that placed it here, and \
                 nothing on this page will change it."))</p>
            <dl class="kv">
                <dt>(t("Placed by"))</dt>
                <dd class="mono">(service.origin_node_id.as_deref().unwrap_or("—"))</dd>
            </dl>

            // What is actually on *this* machine. The page used to name
            // the node that sent it and stop there, so the operator
            // could not see how many copies they were running or what
            // any of them was doing — which is the one thing they can
            // see about it that the other node cannot.
            <table>
                <thead>
                    <tr><th>(t("Running here"))</th><th>(t("State"))</th></tr>
                </thead>
                <tbody>
                    @for replica in mine {
                        <tr>
                            <td class="mono">("#")(replica.slot)</td>
                            <td data-replica=(&replica.id)>
                                (placement_state(
                                    replica,
                                    service.desired_state == crate::entities::service::DesiredState::Stopped,
                                    false,
                                    // Whoever routes this service knows
                                    // whether it answers, and for one
                                    // that arrived on an errand that is
                                    // the node which placed it.
                                    false,
                                ))
                            </td>
                        </tr>
                    }
                </tbody>
            </table>
            <p class="field-hint">(t("The machine is yours: throwing it out is something you can always do, \
                 and it is the only thing here that is."))</p>
        </section>

        <section class="card stack">
            <p class="card-label">(t("Throw it off this node"))</p>
            @if evicted {
                <p>(t("Already thrown out. Its containers are stopped and the node that \
                     placed it has been told — or will be, the next time it asks."))</p>
                <p class="field-hint">(t("The rows stay because they are what carries that news. Removing \
                     them here would leave the other node sending the same instruction \
                     again."))</p>
            } @else {
                <p>(t("Stops its containers here and tells the node that placed it to \
                     stop asking. It cannot be undone from this side — that node \
                     decides what happens next, and it may place it somewhere else."))</p>
                <form method="post" action=(format!(
                    "/projects/{}/services/{}/evict", project.slug, service.slug
                ))>
                    <button class="btn btn-ghost destructive" type="submit">
                        (t("Throw it out"))
                    </button>
                </form>
            }
        </section>
    }
}
