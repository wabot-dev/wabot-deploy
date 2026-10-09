//! What an organisation has undertaken, against what it may.

use hypertext::prelude::*;

use crate::domain::entities::tenant::Tenant;
use crate::domain::policies::presets;
use crate::features::organisations::texts::t;
use crate::infra::deploy::quotas::Charged;
use crate::shell::texts::sentences;

/// One resource: the ceiling, what is spoken for, and the bar between.
///
/// **The bar is drawn only where there is a ceiling.** A meter against
/// no limit is a bar that can never fill, which reads as "plenty left"
/// on an organisation nothing is stopping.
fn row(
    label: &str,
    used: String,
    limit: Option<String>,
    share: Option<u8>,
) -> impl Renderable + '_ {
    rsx! {
        <div class="stack-sm">
            <div class="split">
                <span class="card-label">(label)</span>
                @if let Some(limit) = &limit {
                    <span class="tile-detail mono">(&used)(" / ")(limit)</span>
                } @else {
                    <span class="tile-detail mono">(&used)(" · ")(t("no ceiling"))</span>
                }
            </div>
            @if let Some(share) = share {
                <div class="meter">
                    <span class="meter-part meter-containers"
                          style=(crate::features::network::node_usage_component::bar_width(f64::from(share)))></span>
                </div>
            }
        </div>
    }
}

/// A share of the ceiling, clamped: **over the ceiling is a full bar and
/// a number that says so**, not a bar drawn past its own box.
fn share(used: u64, limit: u64) -> u8 {
    match limit {
        0 => 100,
        limit => ((used.min(limit) * 100) / limit) as u8,
    }
}

pub fn quota_card<'a>(tenant: &'a Tenant, charged: &'a Charged) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("What it has taken"))</p>
            // **Everywhere, not here.** The sum is over every copy of
            // every service of this organisation wherever it runs, which
            // is what a quota on an organisation has to mean — a ceiling
            // that only counted this machine would be one somebody
            // steps over by placing a copy on the next node.
            <p class="field-hint">(t("Summed over every copy of every service, on this node and \
                 on any other that runs one. What a service may take is what counts — the \
                 ceiling is the reservation."))</p>

            (row(t("Memory"), crate::infra::node::memory::human(charged.memory),
                 tenant.quota_memory.map(crate::infra::node::memory::human),
                 tenant.quota_memory.map(|limit| share(charged.memory, limit))))
            (row(t("CPU"), presets::cpu_label(charged.cpu),
                 tenant.quota_cpu.map(presets::cpu_label),
                 tenant.quota_cpu.map(|limit| share(charged.cpu as u64, limit as u64))))
            (row(t("Disk"), presets::label(charged.disk),
                 tenant.quota_disk.map(presets::label),
                 tenant.quota_disk.map(|limit| share(charged.disk, limit))))

            // **A quota demands ceilings.** A service with no
            // `memory_limit` is unmeasurable, so an organisation with a
            // quota and one of those has a quota that is decoration.
            // Named here rather than only in the refusal somebody meets
            // later.
            @if charged.unbounded > 0 && tenant.quota_memory.is_some() {
                <p class="failure">(sentences::services_without_a_ceiling(charged.unbounded))</p>
            }
        </section>
    }
}
