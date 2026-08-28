//! What this service is doing here: its copies, and what they are using.

use crate::views::texts::language::t;
use hypertext::prelude::*;

/// What each copy is doing and what it is using, here.
///
/// The detail page's half of what used to be one card: a table is what
/// somebody opens this page to read, and the selectors that change a
/// placement are a decision, which is settings' half.
///
/// Memory is per replica and only for the copies on **this** machine. A
/// copy elsewhere is measured by the node running it and reports its state
/// back, not its cgroup — so this says nothing rather than guessing, which
/// is the same rule the log page follows about a copy it cannot read.
/// Where this table is, and what it measured there.
///
/// Memory comes from the cgroups of the copies on this machine; disk from
/// walking their volumes. A copy elsewhere is in neither map and falls back
/// to what its own node last reported.
pub struct Running<'a> {
    pub project_slug: &'a str,
    pub service_slug: &'a str,
    pub used: &'a std::collections::BTreeMap<String, u64>,
    pub disk: &'a std::collections::BTreeMap<String, Option<u64>>,
    /// The copies this node's edge has taken out of the rotation, by
    /// replica id. Empty when this node routes nothing for the service,
    /// which is every node that is not an edge for it — health is known
    /// by whoever proxies, and nobody else has an opinion to offer.
    pub silent: &'a [String],
}

pub fn running_card<'a>(
    placements: &'a [crate::entities::replica::Replica],
    nodes: &'a [crate::network::Node],
    queued: &'a [String],
    stopped: bool,
    // The ids this table is about, and what each copy here is using. One
    // struct because they travel together and seven bare arguments is a
    // call nobody can read — which the linter says with a number and the
    // next reader would say with a sigh.
    at: &'a Running<'a>,
) -> impl Renderable + 'a {
    let live: Vec<&crate::entities::replica::Replica> =
        placements.iter().filter(|r| !r.evicted()).collect();
    // This node's own, which is the figure somebody sizing this machine
    // wants — a total across the network would answer a question nobody
    // asked while hiding the one they did.
    let here: u64 = live
        .iter()
        .filter_map(|replica| {
            at.used
                .get(&replica.container_id(at.project_slug, at.service_slug))
                .copied()
        })
        .sum();

    rsx! {
        <section class="stack">
            <div class="split">
                <p class="card-label">(t("Where this runs"))</p>
                <span class="who">
                    (format!("{} replica(s)", live.len()))
                    @if here > 0 {
                        (" · ")(crate::node::memory::human(here))(t(" here"))
                    }
                </span>
            </div>

            <div class="card stack">
                <table>
                    <thead>
                        <tr>
                            <th>(t("Replica"))</th>
                            <th>(t("Node"))</th>
                            <th>(t("State"))</th>
                            <th>(t("CPU"))</th>
                            <th>(t("Memory"))</th>
                            <th>(t("Disk"))</th>
                        </tr>
                    </thead>
                    <tbody>
                        @for replica in &live {
                            <tr>
                                <td class="mono">("#")(replica.slot)</td>
                                <td class="mono">(crate::views::components::service_placement::node_name(replica, nodes))</td>
                                <td data-replica=(&replica.id)>
                                    (crate::views::components::service_placement::placement_state(
                                        replica,
                                        stopped,
                                        replica.node_id.as_ref().is_some_and(|node| {
                                            queued.iter().any(|q| q == node)
                                        }),
                                        at.silent.contains(&replica.id),
                                    ))
                                </td>
                                // A copy here is read from its cgroup; one
                                // elsewhere is whatever its own node last
                                // reported, which is the only way that
                                // figure can exist on this machine. Still
                                // "—" when neither has one, because a
                                // database using no memory is not a
                                // reading anybody should believe.
                                // For a copy here the stream writes this,
                                // because it has the two readings a rate
                                // needs; for one elsewhere the server
                                // renders what that node worked out and
                                // sent, and the stream leaves it alone.
                                //
                                // The two are measured over different
                                // windows — two seconds against a report
                                // interval — which the card below says
                                // rather than letting somebody read the
                                // remote copy as the calmer one.
                                <td class="mono" data-cpu=(&replica.id)>(
                                    replica
                                        .cpu_millicores
                                        .filter(|_| !replica.is_here())
                                        .map(|milli| format!("{milli}m"))
                                        .unwrap_or_default()
                                )</td>
                                <td class="mono">(
                                    at.used.get(&replica.container_id(at.project_slug, at.service_slug))
                                        .copied()
                                        .or(replica.memory_bytes)
                                        .map(crate::node::memory::human)
                                        .unwrap_or_else(|| "—".into())
                                )</td>
                                // What its volume holds. The figure that
                                // matters most for a database and the one
                                // nothing else watches: a memory ceiling
                                // is a decision somebody made, and a
                                // volume grows until the machine is full.
                                <td class="mono">(
                                    at.disk.get(&replica.id)
                                        .copied()
                                        .flatten()
                                        .or(replica.disk_bytes)
                                        .map(crate::node::memory::human)
                                        .unwrap_or_else(|| "—".into())
                                )</td>
                            </tr>
                        }
                    </tbody>
                </table>
                @if live.iter().any(|replica| !replica.is_here()) {
                    <p class="field-hint">(t("A copy here is measured every couple of seconds; \
                         one on another node is measured by that node, across the interval it \
                         reports on. The same unit, and the second is the smoother of the \
                         two."))</p>
                }
            </div>
        </section>
    }
}

/// The disks themselves: one per copy, and that is the whole point.
///
/// The table above measures them and never says what they *are* — so a
/// database's storage was a number in a column and nothing that said
/// there is a directory on a machine, one for each copy, that outlives
/// every deployment. Asked for by name.
///
/// One row per copy rather than one per volume-and-copy, because a
/// service declares one volume today and the row would be the same
/// sentence repeated. The paths are listed in the cell when there are
/// several, and the size is the copy's total either way — which is the
/// figure its own node reports, and the only one that can exist here
/// for a copy on another machine.
pub fn disks_card<'a>(
    volumes: &'a [crate::entities::volume::Volume],
    placements: &'a [crate::entities::replica::Replica],
    nodes: &'a [crate::network::Node],
    at: &'a Running<'a>,
) -> impl Renderable + 'a {
    let live: Vec<&crate::entities::replica::Replica> =
        placements.iter().filter(|r| !r.evicted()).collect();
    let paths = volumes
        .iter()
        .map(|volume| volume.path.clone())
        .collect::<Vec<_>>()
        .join("  ");

    rsx! {
        <section class="stack">
            <p class="card-label">(t("Persistent disks"))</p>
            <div class="card stack">
                <table>
                    <thead>
                        <tr>
                            <th>(t("Replica"))</th>
                            <th>(t("Node"))</th>
                            <th>(t("Inside the container"))</th>
                            <th>(t("Holds"))</th>
                        </tr>
                    </thead>
                    <tbody>
                        @for replica in &live {
                            <tr>
                                <td class="mono">("#")(replica.slot)</td>
                                <td class="mono">(crate::views::components::service_placement::node_name(replica, nodes))</td>
                                <td class="mono">(&paths)</td>
                                <td class="mono">(
                                    at.disk.get(&replica.id)
                                        .copied()
                                        .flatten()
                                        .or(replica.disk_bytes)
                                        .map(crate::node::memory::human)
                                        .unwrap_or_else(|| "—".into())
                                )</td>
                            </tr>
                        }
                    </tbody>
                </table>
                <p class="field-hint">(t("A disk belongs to one copy and survives every \
                     deployment — a container is replaced, the directory under it is not. \
                     Two copies never share one: that would be two servers writing the same \
                     files. It goes when the copy does, and a copy that comes back gets an \
                     empty one."))</p>
            </div>
        </section>
    }
}
