use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::policies::presets;
use crate::infra::deploy::Deployer;

/// **The floor is a measurement, not a constant.** A reserve under what
/// this node's own processes are using right now — this binary,
/// containerd, the shims — is refused with the reading, because the node
/// can answer that question about itself and 256 MB cannot.
///
/// And a reserve that leaves less than is already promised is refused
/// too, naming the figure: the ceilings on existing services are
/// reservations, so accepting it would make every "does this fit" answer
/// on the node wrong until somebody deleted something.
pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    memory: &str,
    cpu: &str,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let memory = presets::parse_size(memory)?;
    let cpu = presets::parse_cores(cpu)?;

    let snapshot = deployer.memory().await;
    let committed = deployer.committed().await;

    if let Some(bytes) = memory {
        // Everything outside anybody's cgroup, which is what a reserve
        // is for.
        let in_use = snapshot.node + snapshot.containerd + snapshot.shims;
        if let Some(reason) = reserve_refusal(
            snapshot.total,
            in_use,
            committed.memory,
            bytes,
            crate::infra::node::memory::human,
        ) {
            return Err(reason);
        }
    }

    if let Some(millicores) = cpu {
        // No floor from a reading here: the node's own CPU use is a rate
        // the stream computes from two samples, and a form has one.
        // `parse_cores` already refuses below a tenth of a core.
        if let Some(reason) = reserve_refusal(
            u64::from(crate::infra::node::cpu::cores() * 1_000),
            0,
            u64::from(committed.cpu),
            u64::from(millicores),
            |millicores| presets::cpu_label(u32::try_from(millicores).unwrap_or(u32::MAX)),
        ) {
            return Err(reason);
        }
    }

    // Logged rather than propagated: a write that fails leaves the
    // previous number in place and the page redraws it, which is a
    // truthful page rather than a 500.
    if let Err(error) = crate::infra::node::settings::set_memory_reserve(database, memory).await {
        tracing::error!(%error, "could not store the memory reserve");
    }
    if let Err(error) = crate::infra::node::settings::set_cpu_reserve(database, cpu).await {
        tracing::error!(%error, "could not store the CPU reserve");
    }
    // Nothing is redeployed. A reserve decides what the node will
    // *accept* next; it does not reach into a container's cgroup, and
    // restarting every service because a number moved would be a setting
    // that costs an outage.
    Ok(())
}

fn reserve_refusal(
    total: u64,
    in_use: u64,
    committed: u64,
    wanted: u64,
    say: impl Fn(u64) -> String,
) -> Option<String> {
    if wanted < in_use {
        return Some(format!(
            "this node is using {} for itself right now — reserving {} would promise memory it is \
             already spending",
            say(in_use),
            say(wanted)
        ));
    }
    let left = total.saturating_sub(wanted);
    match left < committed {
        true => Some(format!(
            "{} is already promised to services, and that would leave {}",
            say(committed),
            say(left)
        )),
        false => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reserve_the_node_cannot_keep_is_refused() {
        let mb = 1024 * 1024;
        let say = |bytes: u64| format!("{} MB", bytes / mb);

        // Under what the node itself is using: refused, and it says the
        // number it measured rather than a rule.
        let refusal = reserve_refusal(1024 * mb, 161 * mb, 0, 6 * mb, say)
            .expect("6 MB is under 161 MB of measured use");
        assert!(refusal.contains("161 MB"), "{refusal}");
        assert!(
            refusal.contains("6 MB"),
            "and what was asked for: {refusal}"
        );

        // Room for what is already promised: refused, naming the promise.
        let refusal = reserve_refusal(1024 * mb, 161 * mb, 700 * mb, 512 * mb, say)
            .expect("512 MB reserved leaves 512 MB against 700 MB promised");
        assert!(refusal.contains("700 MB"), "{refusal}");
        assert!(
            refusal.contains("512 MB"),
            "and what would be left: {refusal}"
        );

        // And a number that fits both is accepted.
        assert_eq!(
            reserve_refusal(1024 * mb, 161 * mb, 300 * mb, 256 * mb, say),
            None
        );
        // Exactly what is in use is enough: the floor is "not less than".
        assert_eq!(reserve_refusal(1024 * mb, 161 * mb, 0, 161 * mb, say), None);
    }
}
