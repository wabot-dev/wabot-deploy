//! What the machine keeps for itself, and what another node may do to it.

use crate::network;
use crate::node::memory::Snapshot;
use crate::views::layouts::layout;
use crate::views::texts::language::t;
use hypertext::prelude::*;

/// What this machine keeps for itself, and what is left for services.
///
/// It was two constants: 15 % of memory with a floor of 256 MB, and a
/// quarter core. Those are a reasonable guess and cannot be right for
/// every machine — deliberately generous on a small node, and on a 32 GB
/// box 15 % is five gigabytes nothing is using. Asked for by Jorge.
///
/// The refusal is the interesting half and it is on the POST: a reserve
/// under what this node's own processes are *measured* using right now is
/// refused with the reading, because the node can answer that question
/// about itself and a constant cannot.
pub fn reserves_card<'a>(
    snapshot: &'a Snapshot,
    reserves: (Option<u64>, Option<u32>),
    cores: u32,
) -> impl Renderable + 'a {
    let (memory, cpu) = reserves;
    let default_memory = crate::policies::presets::default_memory_reserve(snapshot.total);
    let memory_value = memory
        .map(crate::policies::presets::size_field)
        .unwrap_or_default();
    let cpu_value = cpu
        .map(crate::policies::presets::cores_field)
        .unwrap_or_default();
    let default_cpu =
        crate::policies::presets::cores_field(crate::policies::presets::NODE_RESERVE_MILLICORES);
    let default_memory_label = crate::policies::presets::label(default_memory);
    let allocatable = crate::node::memory::human(crate::policies::presets::allocatable_memory(
        snapshot.total,
        memory,
    ));
    let allocatable_cpu = crate::policies::presets::cpu_label(
        crate::policies::presets::allocatable_cpu(cores * 1_000, cpu),
    );

    rsx! {
        <section class="card stack" id="reserves">
            <p class="card-label">(t("Reserved for the node"))</p>
            <form method="post" action="/node/reserves" class="stack">
                <label for="memory-reserve">(t("Memory"))</label>
                <input id="memory-reserve" name="memory" type="text" autocomplete="off"
                       placeholder=(&default_memory_label) value=(&memory_value)>
                <p class="field-hint">(t("Kept for the console, the edge, containerd and \
                     everything else on this machine. Empty means the default, which is 15 % \
                     with a floor of 256 MB."))</p>

                <label for="cpu-reserve">(t("CPU"))</label>
                <input id="cpu-reserve" name="cpu" type="text" autocomplete="off"
                       placeholder=(&default_cpu) value=(&cpu_value)>
                <p class="field-hint">(t("Cores, like 0.5, or millicores with an m. Empty means \
                     the default of a quarter core."))</p>

                // The consequence, in the units the rest of the console
                // uses. A reserve is only meaningful as "what is left",
                // and an operator typing a number should not have to do
                // the subtraction to find out what they did.
                <p class="field-hint">(t("Leaves "))(&allocatable)(t(" and "))
                    (&allocatable_cpu)(t(" for services."))</p>
                <div class="actions">
                    <button type="submit">(t("Save"))</button>
                </div>
            </form>
        </section>
    }
}

/// What that node is running, and what it has been asked to run.
///
/// The version is a **learned** fact and the wanted one is a decision, so
/// the card says both and never conflates them: "asked for v0.7.1, still
/// on 0.6.6" is the honest reading of a node that has not collected yet,
/// of one that is off, and of one that refused — and the reason for the
/// third is on the errand row below.
pub fn manage_card<'a>(
    node: &'a network::Node,
    releases: &'a [String],
    catalogue_failed: bool,
) -> impl Renderable + 'a {
    let may_manage = node
        .allows
        .contains(&crate::values::capability::Capability::Manage);
    let behind = node.wanted_version.as_deref().is_some_and(|wanted| {
        node.version
            .as_deref()
            .is_none_or(|running| !crate::update::is_release(wanted, running))
    });
    rsx! {
        <section class="stack">
            <p class="card-label">(t("Keeping it up to date"))</p>
            <div class="card stack">
                <dl class="kv">
                    <dt>(t("Running"))</dt>
                    <dd class="mono">(node.version.as_deref().unwrap_or("—"))</dd>
                    @if let Some(wanted) = &node.wanted_version {
                        <dt>(t("Asked for"))</dt>
                        <dd class="mono">(wanted)</dd>
                    }
                </dl>
                @if node.version.is_none() {
                    <p class="note">(t("It has not said. A node that has never reported a version is \
                         either older than this or has not been heard from — asking \
                         it to catch up is what finds out which."))</p>
                }
                // The failure that cannot travel on the errand: it is
                // settled before the install starts, because the install
                // ends by replacing the process that would settle it.
                @if let Some(note) = node.update_note.as_deref().filter(|note| !note.is_empty()) {
                    (layout::error_note(note))
                }
                @if behind {
                    <p class="note">(t("It is not there yet. The instruction is collected on that \
                         node's own schedule, and a node that is off collects it \
                         when it comes back."))</p>
                }
            </div>

            @if !may_manage {
                <div class="card stack">
                    <p class="tile-detail">(t("That node has not agreed to be kept up to date from \
                         here. It is granted on the node itself, from its own \
                         console — a grant belongs to the machine it binds."))</p>
                </div>
            } @else {
                <form method="post" action=(format!("/network/{}/version", node.id))
                      class="card stack">
                    <label class="field">
                        <span class="field-label">(t("Release"))</span>
                        <select name="version">
                            <option value="">(t("— no version asked for —"))</option>
                            @for tag in releases {
                                @if node.wanted_version.as_deref() == Some(tag.as_str()) {
                                    <option value=(tag) selected>(tag)</option>
                                } @else {
                                    <option value=(tag)>(tag)</option>
                                }
                            }
                        </select>
                    </label>
                    @if catalogue_failed {
                        <p class="field-hint">(t("The release list could not be read just now, so this \
                             is empty. Nothing about that node has changed."))</p>
                    } @else {
                        <p class="field-hint">(t("It resolves the tag itself, against the same list its \
                             own console reads — so a release with no build for that \
                             machine is refused there, with the reason on the \
                             instruction below."))</p>
                    }
                    <div class="actions">
                        <button type="submit">(t("Ask for this release"))</button>
                    </div>
                </form>

                <form method="post" action=(format!("/network/{}/command", node.id))
                      class="card stack">
                    <p class="card-label">(t("Once, now"))</p>
                    <p class="field-hint">(t("Neither of these is a state it converges on: asked \
                         twice, it happens twice. Restarting replaces the process \
                         and leaves the containers running."))</p>
                    <div class="actions">
                        <button class="btn btn-ghost" type="submit" name="action" value="restart">
                            (t("Restart it"))
                        </button>
                        <button class="btn btn-ghost" type="submit" name="action" value="back-up">
                            (t("Back it up now"))
                        </button>
                    </div>
                </form>
            }
        </section>
    }
}
