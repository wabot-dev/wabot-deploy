//! The kernel settings this node's engines need, and nothing else.
//!
//! One today: `vm.overcommit_memory = 1`. A Valkey store forks to rewrite
//! its AOF and to hand a backup its snapshot, and the child shares every
//! page with the parent until one is written. Under the heuristic the
//! kernel defaults to, that fork can be refused on a machine with too little
//! free memory to have copied everything — which it never does — and the
//! server then logs that it cannot save and carries on without. The ceiling
//! that matters is the container's cgroup, which this does not change.

use std::path::Path;

pub const OVERCOMMIT: &str = "/proc/sys/vm/overcommit_memory";

/// Read at boot by `systemd-sysctl` and by OpenRC's `sysctl` service.
pub const SYSCTL_FILE: &str = "/etc/sysctl.d/90-wabot-deploy.conf";

const WANTED: &str = "vm.overcommit_memory = 1\n";

pub fn overcommit() -> Option<u8> {
    std::fs::read_to_string(OVERCOMMIT)
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Convergent: it asks the kernel and the file, not the ledger.
pub fn ensure() -> std::io::Result<String> {
    if cfg!(test) {
        return Err(std::io::Error::other(
            "a test does not change the machine's kernel settings",
        ));
    }
    let kept = std::fs::read_to_string(SYSCTL_FILE).is_ok_and(|text| text == WANTED);
    let live = overcommit() == Some(1);
    if kept && live {
        return Ok("vm.overcommit_memory is already 1".into());
    }
    if !kept {
        if let Some(parent) = Path::new(SYSCTL_FILE).parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(SYSCTL_FILE, WANTED)?;
    }
    if !live {
        std::fs::write(OVERCOMMIT, "1")?;
    }
    Ok(format!("vm.overcommit_memory = 1, kept in {SYSCTL_FILE}"))
}

/// The line `doctor` prints.
pub fn describe(value: Option<u8>) -> String {
    match value {
        Some(1) => "overcommit 1".into(),
        Some(other) => format!(
            "overcommit {other} — a Valkey store's backup may be refused the fork it needs; \
             `wabot-deploy install` sets it to 1"
        ),
        None => format!("overcommit unknown — {OVERCOMMIT} could not be read"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_never_changes_the_kernel() {
        assert!(ensure().is_err());
    }

    #[test]
    fn anything_but_one_says_what_it_costs_and_how_to_fix_it() {
        assert_eq!(describe(Some(1)), "overcommit 1");
        for value in [Some(0), Some(2)] {
            let said = describe(value);
            assert!(
                said.contains("Valkey") && said.contains("install"),
                "{said}"
            );
        }
        assert!(describe(None).contains(OVERCOMMIT));
    }
}
