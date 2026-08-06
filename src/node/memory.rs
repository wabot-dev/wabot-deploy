//! Where the RAM went.
//!
//! ## Read from `/proc` and the cgroup tree, not from a crate
//!
//! Both are stable kernel interfaces with a documented format, and
//! parsing the four lines this needs is shorter than the dependency
//! that would parse all of them. It also keeps the reading honest
//! about which numbers are the kernel's and which are ours.
//!
//! ## The parts do not add up, and saying so is the point
//!
//! A container's `memory.current` includes its page cache, which is
//! also counted in the system's `Cached` — and `Cached` is part of
//! what `MemAvailable` says you can have back. Process RSS
//! double-counts shared pages between the processes that share them.
//! So the shares below overlap, and "everything else" is a remainder
//! rather than a measurement.
//!
//! The alternative is a number that adds up exactly and means nothing.
//! What an operator needs is the order of magnitude of each part —
//! is the platform costing 30 MB or 300 — and for that, overlap in the
//! third significant figure is not the problem.

use std::collections::BTreeMap;

/// One reading of the machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Snapshot {
    /// Every figure is bytes.
    pub total: u64,
    /// What the kernel says can be handed out without swapping —
    /// which is not `free`, because reclaimable cache is available.
    pub available: u64,
    pub free: u64,
    pub cached: u64,
    pub swap_total: u64,
    pub swap_used: u64,

    /// This process.
    pub node: u64,
    /// containerd, if it is running here.
    pub containerd: u64,
    /// The shims, one per running container.
    pub shims: u64,
    pub shim_count: usize,
    /// Each running container, by its id.
    pub containers: BTreeMap<String, u64>,
}

impl Snapshot {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }

    pub fn containers_total(&self) -> u64 {
        self.containers.values().sum()
    }

    /// The platform's own cost: this process, containerd, the shims.
    pub fn platform(&self) -> u64 {
        self.node + self.containerd + self.shims
    }

    /// What is left once the platform and the containers are taken
    /// out. A remainder, not a measurement — see the module docs.
    pub fn rest(&self) -> u64 {
        self.used()
            .saturating_sub(self.platform())
            .saturating_sub(self.containers_total())
    }

    /// A share of the total, for a bar. Never above 100.
    pub fn percent_of_total(&self, bytes: u64) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        ((bytes as f64 / self.total as f64) * 100.0).min(100.0)
    }
}

/// Read the machine, attributing what can be attributed.
///
/// `container_pids` maps a container id to the pid of its task; the
/// caller has them from containerd. A container's cgroup is found
/// through its pid rather than by guessing the path, because the
/// layout depends on the cgroup driver and the guess would be wrong
/// on exactly the machines that are configured unusually.
pub fn read(container_pids: &BTreeMap<String, u32>) -> Snapshot {
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let field = |name: &str| meminfo_field(&meminfo, name);

    let swap_total = field("SwapTotal");
    let mut snapshot = Snapshot {
        total: field("MemTotal"),
        available: field("MemAvailable"),
        free: field("MemFree"),
        cached: field("Cached"),
        swap_total,
        swap_used: swap_total.saturating_sub(field("SwapFree")),
        node: rss_of(std::process::id()).unwrap_or(0),
        ..Default::default()
    };

    for (name, pid) in processes() {
        match name.as_str() {
            "containerd" => snapshot.containerd += rss_of(pid).unwrap_or(0),
            // The comm field is truncated to 15 bytes, so the shim is
            // `containerd-shim` here and never its full name. Matching
            // the full name finds nothing, silently.
            "containerd-shim" => {
                snapshot.shims += rss_of(pid).unwrap_or(0);
                snapshot.shim_count += 1;
            }
            _ => {}
        }
    }

    for (id, pid) in container_pids {
        if let Some(bytes) = cgroup_memory(*pid) {
            snapshot.containers.insert(id.clone(), bytes);
        }
    }

    snapshot
}

/// A `/proc/meminfo` line, in bytes.
///
/// The file reports kB — kibibytes, despite the label — for every
/// field except a couple that carry no unit. Multiplying by 1024 is
/// what every reader of this file does.
fn meminfo_field(meminfo: &str, name: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            if key != name {
                return None;
            }
            let value = value.trim();
            let number: u64 = value.split_whitespace().next()?.parse().ok()?;
            Some(if value.ends_with("kB") {
                number * 1024
            } else {
                number
            })
        })
        .unwrap_or(0)
}

/// Resident set size of a process, in bytes.
fn rss_of(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    Some(meminfo_field(&status, "VmRSS"))
}

/// Every process, as (comm, pid).
fn processes() -> Vec<(String, u32)> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
            Some((comm.trim().to_string(), pid))
        })
        .collect()
}

/// What the cgroup holding this pid is currently using.
///
/// cgroup v2 only: `memory.current` is a v2 file, and the node
/// requires a unified hierarchy at install time — the preflight check
/// refuses a machine without one, so there is no v1 path to support.
fn cgroup_memory(pid: u32) -> Option<u64> {
    let cgroup = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    // `0::/system.slice/crun-project.service.scope/container` — the
    // v2 line is the one with an empty controller list.
    let path = cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?
        .trim();

    let current = std::fs::read_to_string(format!("/sys/fs/cgroup{path}/memory.current")).ok()?;
    current.trim().parse().ok()
}

/// Bytes as somebody would say them out loud.
///
/// Binary units, because that is what `/proc` reports and what every
/// other tool on the machine will show — a console that said 1.05 GB
/// where `free` says 1.0 GiB would be right and useless.
pub fn human(bytes: u64) -> String {
    const UNITS: [(&str, u64); 4] = [
        ("GB", 1024 * 1024 * 1024),
        ("MB", 1024 * 1024),
        ("kB", 1024),
        ("B", 1),
    ];

    for (unit, size) in UNITS {
        if bytes >= size {
            let value = bytes as f64 / size as f64;
            // One decimal below 10, none above: "9.8 GB" and "512 MB"
            // both read as numbers rather than as measurements.
            return if value < 10.0 && size > 1 {
                format!("{value:.1} {unit}")
            } else {
                format!("{} {unit}", value.round())
            };
        }
    }
    "0 B".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "\
MemTotal:         863408 kB
MemFree:          101648 kB
MemAvailable:     491928 kB
Buffers:            4780 kB
Cached:           494520 kB
SwapTotal:       2097148 kB
SwapFree:        1922556 kB
HugePages_Total:       0
";

    #[test]
    fn meminfo_is_read_in_bytes() {
        assert_eq!(meminfo_field(MEMINFO, "MemTotal"), 863_408 * 1024);
        assert_eq!(meminfo_field(MEMINFO, "MemAvailable"), 491_928 * 1024);
        assert_eq!(meminfo_field(MEMINFO, "Nothing"), 0);
    }

    /// A field without a unit must not be multiplied. `HugePages_Total`
    /// is a count, and treating it as kilobytes would be off by 1024.
    #[test]
    fn a_field_with_no_unit_is_taken_as_it_is() {
        assert_eq!(meminfo_field(MEMINFO, "HugePages_Total"), 0);
        assert_eq!(meminfo_field("Threads:\t12\n", "Threads"), 12);
    }

    #[test]
    fn used_is_what_the_kernel_cannot_hand_back() {
        let snapshot = Snapshot {
            total: 1000,
            available: 400,
            ..Default::default()
        };
        // Not total - free: reclaimable cache is available, and
        // counting it as used is how a healthy machine looks full.
        assert_eq!(snapshot.used(), 600);
    }

    #[test]
    fn the_platforms_share_is_its_three_parts() {
        let snapshot = Snapshot {
            node: 12,
            containerd: 30,
            shims: 22,
            ..Default::default()
        };
        assert_eq!(snapshot.platform(), 64);
    }

    /// The parts overlap — a container's page cache is counted twice —
    /// so the remainder has to floor at zero rather than wrap.
    #[test]
    fn the_remainder_never_goes_below_zero() {
        let snapshot = Snapshot {
            total: 1000,
            available: 990,
            node: 500,
            ..Default::default()
        };
        assert_eq!(snapshot.used(), 10);
        assert_eq!(snapshot.rest(), 0, "not an underflow of u64");
    }

    #[test]
    fn a_share_of_nothing_is_not_a_division_by_zero() {
        assert_eq!(Snapshot::default().percent_of_total(100), 0.0);
    }

    #[test]
    fn a_share_never_exceeds_the_whole() {
        let snapshot = Snapshot {
            total: 100,
            ..Default::default()
        };
        assert_eq!(snapshot.percent_of_total(500), 100.0);
        assert_eq!(snapshot.percent_of_total(25), 25.0);
    }

    #[test]
    fn bytes_read_the_way_a_person_says_them() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(900), "900 B");
        assert_eq!(human(12 * 1024), "12 kB");
        assert_eq!(human(11_534_336), "11 MB");
        assert_eq!(human(1_610_612_736), "1.5 GB");
        // Below ten, the decimal is the difference between "1 GB" and
        // "1.9 GB", which is nearly twice as much.
        assert_eq!(human(9 * 1024 * 1024), "9.0 MB");
    }

    /// The real reading, on whatever machine runs the tests. It cannot
    /// assert numbers, but it can assert that the parsing found any at
    /// all — a `/proc` that moved would otherwise show as zeros in a
    /// console nobody is watching.
    #[test]
    #[cfg(target_os = "linux")]
    fn a_reading_of_this_machine_finds_something() {
        let snapshot = read(&BTreeMap::new());

        assert!(snapshot.total > 0, "no MemTotal");
        assert!(snapshot.node > 0, "this process has no RSS");
        assert!(snapshot.available <= snapshot.total);
    }
}
