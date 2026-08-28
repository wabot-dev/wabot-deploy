//! What the machine is using, and what it is reachable at.

use crate::network;
use crate::network::Kind;
use crate::node::memory;
use crate::node::memory::Snapshot;
use crate::views::texts::language::t;
use hypertext::prelude::*;

/// What a node is, in the one line the list has room for.
///
/// Said as what it *does*, not as a category. "Private" is a word this
/// console printed at somebody and expected them to know it meant "does
/// not answer for names" — and since the capability switch it is not
/// even a property of the machine, it is a decision. So the page says
/// the decision.
pub fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Public => t("answers for names"),
        Kind::Private => t("runs containers only"),
    }
}

/// How this node reaches another one — or, for itself, what it is.
///
/// Two questions, and they were being answered by one column. A node
/// with a public address that joins a hub reads `public` on its own
/// page, because it is, and the hub had it listed as `private` at the
/// same moment. Both rows were right and the pair was nonsense: the
/// hub does not know what that machine is, only that it has nothing
/// but an overlay address for it.
///
/// So a row about somebody else describes the *relationship*. Found by
/// looking at the two consoles side by side, which is the only place
/// it is visible.
pub fn reach(node: &network::Node) -> String {
    if node.is_self {
        return kind_word(node.kind).to_string();
    }
    match &node.overlay_ip {
        // Where it lives on the overlay, which is how everything
        // reaches it: an errand over its control plane, a request from
        // an edge to a container it runs.
        Some(address) => format!("on the overlay at {address}"),
        None => "no address for it yet".to_string(),
    }
}

/// What is known about a node's place on the overlay.
///
/// The same card for this node and for a joined one, because it is the
/// same set of facts: an id other nodes use, an address, a key, and
/// where this node would reach it. A node that has never enrolled
/// anybody and never joined anything has neither of the middle two, and
/// says so rather than showing an empty row.
/// How full another machine is, from what it last reported.
///
/// This node's own page has meters, read live; a node it cannot reach had
/// nothing at all — the report carried replicas, an endpoint and a list of
/// permissions and said nothing about the machine. So the network area
/// answered every question except the one somebody opens it with.
///
/// Two lines rather than the meter and its five parts. The parts are that
/// machine's internals, its own console draws them, and no decision here
/// turns on which process holds which megabyte. Whether it is running out
/// is the thing another node needs.
///
/// A node that has not reported since these columns existed says so,
/// rather than showing nought — which would read as an idle machine.
pub fn usage_card<'a>(node: &'a network::Node, seen: Option<String>) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("How full it is"))</p>
            @if let Some(usage) = node.usage {
                <dl class="kv">
                    <dt>(t("Memory"))</dt>
                    <dd class="mono">
                        (crate::node::memory::human(usage.memory_used))(t(" of "))
                        (crate::node::memory::human(usage.memory_total))
                    </dd>
                    <dt>(t("Disk"))</dt>
                    <dd class="mono">
                        (crate::node::memory::human(
                            usage.disk_total.saturating_sub(usage.disk_free)
                        ))(t(" of "))
                        (crate::node::memory::human(usage.disk_total))
                        (" · ")
                        (crate::node::memory::human(usage.disk_free))(t(" free"))
                    </dd>
                </dl>
                // When, because a reading with no time on it is one
                // somebody trusts after the node has stopped reporting.
                @if let Some(seen) = &seen {
                    <p class="field-hint">(t("As reported "))(seen)(".")</p>
                }
            } @else {
                <p class="field-hint">(t("Nothing yet. A node reports this every fifteen \
                     seconds once it is running a version that sends it — until then this \
                     says nothing rather than nought, which would read as an idle \
                     machine."))</p>
            }
        </section>
    }
}

pub fn network_card(node: &network::Node) -> impl Renderable + '_ {
    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("On this network"))</p>
                <span class="badge badge-info">
                    <span class="dot dot-info"></span>(reach(node))
                </span>
            </div>

            <dl class="kv">
                <dt>(t("Id"))</dt>
                <dd class="mono">(&node.id)</dd>
                // "Reachable at" and not "endpoint": the empty case is
                // the interesting one, and it means this node has no
                // address to dial that one at — not that the machine
                // is unreachable, which is not something this node
                // knows. See `reach`.
                <dt>(t("This node dials it at"))</dt>
                <dd class="mono">(node.endpoint.as_deref().unwrap_or("—"))</dd>
                <dt>(t("Overlay address"))</dt>
                <dd class="mono">(node.overlay_ip.as_deref().unwrap_or("not on one"))</dd>
                <dt>(t("Public key"))</dt>
                <dd class="mono">(node.public_key.as_deref().unwrap_or("none yet"))</dd>
            </dl>

            @if node.is_self {
                <p class="note">(t("A key and an address appear the first time this node enrols \
                     another one or joins one itself. The overlay is what carries \
                     traffic between nodes — an edge here reaching a container that \
                     runs somewhere else."))</p>
            } @else {
                <p class="note">(t("Recorded when this node joined — what it said about itself when it \
                     arrived. Instructions do not travel over the overlay: that node \
                     collects them over the same connection it enrolled through, which \
                     is why nothing here has to be able to reach it."))</p>
            }
        </section>
    }
}

pub fn memory_card<'a>(
    snapshot: &'a Snapshot,
    // What this node has promised, against what it may promise. A
    // ceiling is a reservation here, so these two decide whether another
    // service fits — and an operator who cannot see them experiences
    // that rule as a form that says no.
    committed: &'a crate::deploy::Committed,
    // What the operator said this machine keeps for itself, memory then
    // CPU, or `None` each for the default. It reaches the "promised of"
    // figure, which is the whole point of the setting being editable.
    reserves: (Option<u64>, Option<u32>),
) -> impl Renderable + 'a {
    let containers = snapshot.containers_total();

    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("Memory"))</p>
                <span class="mono" data-cell="summary">
                    (memory::human(snapshot.used()))(t(" of "))(memory::human(snapshot.total))
                </span>
            </div>

            <div class="meter" data-meter>
                <span class="meter-part meter-node"
                      style=(width(snapshot, snapshot.node)) data-bar="node"></span>
                <span class="meter-part meter-runtime"
                      style=(width(snapshot, snapshot.containerd + snapshot.shims))
                      data-bar="runtime"></span>
                <span class="meter-part meter-containers"
                      style=(width(snapshot, containers)) data-bar="containers"></span>
                <span class="meter-part meter-rest"
                      style=(width(snapshot, snapshot.rest())) data-bar="rest"></span>
            </div>

            <table class="mem">
                <tbody>
                    // The names are names — `wabot-deploy`, `containerd`,
                    // a shim — and stay as they are on any machine. The
                    // sentences beside them are prose, and used to render
                    // in English inside a console that was otherwise in
                    // Spanish, because they reach `row` as values and
                    // never went through `t`.
                    (row("node", "wabot-deploy", "meter-node", snapshot.node,
                         t("The console, the edge and the deploy path — this process.")))
                    (row("containerd", "containerd", "meter-runtime", snapshot.containerd,
                         t("The container runtime, shared by every service.")))
                    (row("shims", &format!("shims ({})", snapshot.shim_count), "meter-runtime",
                         snapshot.shims,
                         t("One per running container. The runtime's overhead, not the image's.")))
                    (row("containers", "containers", "meter-containers", containers,
                         t("What the images themselves are using, from their cgroups.")))
                    (row("rest", t("everything else"), "meter-rest", snapshot.rest(),
                         t("The kernel, the distribution, and anything else on this machine.")))
                </tbody>
            </table>

            <p class="note">(t("The parts overlap slightly: a container's page cache counts both in its \
                 own reading and in the system's, and shared pages count for each process \
                 that maps them. \"Everything else\" is what is left over rather than a \
                 measurement of its own."))</p>

            // CPU arrives on the stream and only from the second tick —
            // a rate is the difference between two readings — so the
            // server renders the dash it will replace rather than
            // nothing, which would be a row that appears two seconds
            // after the page. The disk is a card of its own below: it
            // has parts, and a line saying how full a filesystem is
            // answers none of the questions somebody has when it is.
            <dl class="kv">
                <dt>(t("CPU"))</dt>
                <dd data-cell="cpu">("—")</dd>
                <dt>(t("Promised"))</dt>
                <dd>
                    (memory::human(committed.memory))(t(" of "))
                    (memory::human(crate::policies::presets::allocatable_memory(snapshot.total, reserves.0)))
                    (" · ")
                    (crate::policies::presets::cpu_label(committed.cpu))(t(" of "))
                    (crate::policies::presets::cpu_label(
                        crate::node::cpu::allocatable_millicores(reserves.1)
                    ))
                </dd>
            </dl>
            // The number that cannot be counted, said rather than
            // folded in. A container with no ceiling may take
            // everything, and a sum that quietly omitted them would be
            // a figure somebody trusts for a decision it cannot
            // support.
            @if committed.unbounded > 0 {
                <p class="field-hint">
                    (committed.unbounded)(t(" copies here have no memory ceiling, so what is \
                         promised is a floor rather than a total — any of them may take what \
                         is left."))
                </p>
            } @else {
                <p class="field-hint">(t("What a service may take is also what is reserved for \
                     it: this node counts what it has promised and refuses to promise more. \
                     What it keeps for itself — the console, the edge, containerd — is already \
                     out of these figures."))</p>
            }

            @if snapshot.swap_total > 0 {
                <dl class="kv">
                    <dt>(t("Swap"))</dt>
                    <dd data-cell="swap">
                        (memory::human(snapshot.swap_used))(t(" of "))
                        (memory::human(snapshot.swap_total))
                    </dd>
                </dl>
            }
        </section>
    }
}

/// The other ceiling a machine has, in the parts it is made of.
///
/// A line saying a filesystem is 57 % full answers none of the questions
/// somebody has when it is: what is taking it, and which of those can be
/// got back. So it is the memory card's shape — a bar in parts and a
/// table with the same colours — and the parts are the three directories
/// this node can name.
///
/// Read once, when the page is drawn. The memory beside it ticks every
/// two seconds and this must not: each part is a directory walk, and
/// walking containerd's tree every two seconds would put this node at
/// the top of its own CPU column.
pub fn disk_card(disk: &crate::node::disk::Breakdown) -> impl Renderable + '_ {
    // Through `bar_width`, which is the only thing that writes one of
    // these. This card had its own closure emitting a bare `7.0000%` —
    // and a `style` attribute wants a *declaration*, so the browser threw
    // the whole thing away and every segment had no width. The bar was
    // flat and empty while the numbers under it were right, which is what
    // makes this kind of fault survive: nothing looks broken, it just
    // looks like nothing is using the disk. Reported by Jorge.
    //
    // The exact mirror of the memory meter's bug, one page over: there a
    // *declaration* was assigned where the CSSOM wanted a value
    // (`style.width = "width:12%"`), here a value was written where the
    // attribute wanted a declaration. One helper now, so the two cannot
    // disagree again.
    let width = |bytes: u64| bar_width(disk.percent_of_total(bytes));

    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("Disk"))</p>
                <span class="mono">
                    (memory::human(disk.filesystem.used()))(t(" of "))
                    (memory::human(disk.filesystem.total))
                    // A separator is punctuation, not prose: it goes
                    // through no table, like a command or a hostname.
                    (" · ")(memory::human(disk.filesystem.available))(t(" free"))
                </span>
            </div>

            <div class="meter">
                <span class="meter-part meter-containers"
                      style=(width(disk.volumes.bytes))></span>
                <span class="meter-part meter-runtime"
                      style=(width(disk.images.bytes))></span>
                <span class="meter-part meter-node"
                      style=(width(disk.node.bytes))></span>
                <span class="meter-part meter-rest" style=(width(disk.rest()))></span>
            </div>

            <table class="mem">
                <tbody>
                    (row("volumes", t("persistent disks"), "meter-containers", disk.volumes.bytes,
                         t("What services keep. One disk per copy — two copies of a database \
                            on one machine are two databases.")))
                    (row("images", "containerd", "meter-runtime", disk.images.bytes,
                         t("The images pulled to this node and the snapshots unpacked from \
                            them. Nothing collects these yet.")))
                    (row("node", "wabot-deploy", "meter-node", disk.node.bytes,
                         t("This node's own: its database, its certificates and the logs it \
                            keeps for each container.")))
                    (row("rest", t("everything else"), "meter-rest", disk.rest(),
                         t("The kernel, the distribution, and anything else on this machine.")))
                </tbody>
            </table>

            @if disk.partial() {
                <p class="note">(t("One of these directories was too big to walk while a page \
                     was waiting, so its figure is a floor and \"everything else\" is the \
                     larger for it."))</p>
            }
        </section>
    }
}

pub fn row<'a>(
    key: &'a str,
    label: &'a str,
    swatch: &'a str,
    bytes: u64,
    detail: &'a str,
) -> impl Renderable + 'a {
    rsx! {
        <tr>
            <td class="mem-key">
                <span class=(format!("swatch {swatch}"))></span>
                (label)
            </td>
            <td class="mono" data-cell=(key)>(memory::human(bytes))</td>
            <td class="tile-detail">(detail)</td>
        </tr>
    }
}

/// One part's share, as a value the CSSOM will take.
///
/// A bare percentage, not a declaration. It used to be `width:12.34%`,
/// which is right as an attribute and **inert** as an assignment: the
/// stream does `bar.style.width = value`, the CSSOM refuses a value it
/// cannot parse, and it refuses silently. So the meter froze at whatever
/// the first paint drew while the figures beside it went on changing —
/// visible only as a page where the numbers moved and the bar did not.
pub fn percent(snapshot: &Snapshot, bytes: u64) -> String {
    format!("{:.2}%", snapshot.percent_of_total(bytes))
}

/// The same share as a declaration, for the attribute in the first paint.
pub fn width(snapshot: &Snapshot, bytes: u64) -> String {
    bar_width(snapshot.percent_of_total(bytes))
}

/// A meter segment's width, as a `style` attribute's whole contents.
///
/// **A declaration, not a value.** `style="7%"` is not CSS and a browser
/// discards the attribute entirely — silently, leaving a bar that renders
/// flat while the figures beside it are correct. Every meter in this
/// console goes through here so that there is one place to be right, and
/// `every_meter_width_is_a_declaration` fails on a second one that does
/// not.
pub fn bar_width(percent: f64) -> String {
    format!("width:{percent:.4}%")
}

pub fn live_cards<'a>(
    node_id: &'a str,
    disk: &'a crate::node::disk::Breakdown,
    committed: &'a crate::deploy::Committed,
    snapshot: &'a Snapshot,
    reserves: (Option<u64>, Option<u32>),
) -> impl Renderable + 'a {
    let inner = rsx! {
        (memory_card(snapshot, committed, reserves))
        (disk_card(disk))
    }
    .render()
    .into_inner();

    // Rendered first, then wrapped: `rsx!` expands to a closure that
    // captures by move, and nesting one inside the island's would have
    // both wanting the same borrows. Same reason as `Frame::render`.
    wabot::ui::hypertext::island(
        "node-live",
        &serde_json::json!({ "node": node_id }),
        hypertext::Raw::dangerously_create(&inner),
    )
}
