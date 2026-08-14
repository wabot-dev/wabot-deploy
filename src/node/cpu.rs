//! How busy this machine is, and which containers made it so.
//!
//! ## A rate needs two readings, and that shapes the whole module
//!
//! The kernel counts CPU *time*: `/proc/stat` in jiffies since boot, and
//! a cgroup's `cpu.stat` in microseconds since the container started.
//! Neither is a percentage, and dividing a running total by uptime
//! answers "how busy has it been since it started" — which is not the
//! question anybody looking at a console is asking.
//!
//! So this reads a [`Sample`] and the caller keeps the previous one: the
//! percentage is the difference between two, over the wall-clock time
//! between them. The node page has a two-second stream and gets it for
//! free; anywhere without one gets `None`, and the page says nothing
//! rather than showing a figure that means something else.
//!
//! ## Millicores, not a percentage
//!
//! A thousand millicores is one core, busy. It is an **absolute** figure,
//! which is the whole reason to prefer it: a container using one core
//! reads `1000m` on a one-core box and on a thirty-two-core box, so two
//! nodes' numbers can be put beside each other. A percentage cannot —
//! "12 %" means a different amount of work on every machine, and this
//! product's whole shape is several machines.
//!
//! The machine's own capacity is its core count times a thousand, which is
//! what the node's figure is shown against.

use std::collections::BTreeMap;
use std::time::Instant;

/// One reading, meaningless alone.
#[derive(Debug, Clone)]
pub struct Sample {
    at: Instant,
    /// Busy microseconds across the whole machine, from `/proc/stat`.
    busy: u64,
    /// Every microsecond the machine has had, busy or idle. The
    /// denominator has to come from the same source as the numerator, or
    /// a wall-clock divisor would count time the kernel was not measuring.
    total: u64,
    /// Per container, by id: microseconds of CPU it has used.
    containers: BTreeMap<String, u64>,
}

/// What two samples say, in millicores.
#[derive(Debug, Clone, Default)]
pub struct Busy {
    /// The whole machine, which can exceed one core's worth.
    pub node: u32,
    /// What the machine has: cores times a thousand.
    pub capacity: u32,
    pub containers: BTreeMap<String, u32>,
}

/// Take a reading.
///
/// `cgroups` maps a container id to its cgroup path under
/// `/sys/fs/cgroup`, which is what the caller already knows from the pid.
pub fn sample(cgroups: &BTreeMap<String, String>) -> Sample {
    let (busy, total) = machine();
    let mut containers = BTreeMap::new();
    for (id, path) in cgroups {
        if let Some(used) = usage_usec(path) {
            containers.insert(id.clone(), used);
        }
    }
    Sample {
        at: Instant::now(),
        busy,
        total,
        containers,
    }
}

/// The difference between two readings.
///
/// `None` when the pair cannot answer: the same instant twice, a counter
/// that went backwards — which is a machine that rebooted or a container
/// that was recreated — or a total that did not move, which would divide
/// by nought.
pub fn between(before: &Sample, after: &Sample) -> Option<Busy> {
    // **Wall clock**, not the machine's total. Millicores are CPU time over
    // real time — a container that used half a second of CPU in one second
    // is 500m whatever else the machine was doing — where a percentage of
    // the machine needs the machine's total as its divisor. Getting this
    // wrong divides by the core count and reads low on a big box.
    let elapsed = after.at.saturating_duration_since(before.at).as_micros();
    if elapsed == 0 {
        return None;
    }
    // A counter that went backwards is a reboot or a container that was
    // recreated, and neither is a rate.
    let _ = after.total.checked_sub(before.total)?;
    let millicores = |used: u64| ((used as u128 * 1000) / elapsed) as u32;

    let mut containers = BTreeMap::new();
    for (id, used) in &after.containers {
        // A container missing from the earlier sample has just started: it
        // has no rate yet, and counting all of its life as this interval's
        // would show it at several cores for one tick.
        let Some(was) = before.containers.get(id) else {
            continue;
        };
        // Exact: a cgroup counts microseconds, so nothing here rests on
        // what a jiffy is.
        if let Some(used) = used.checked_sub(*was) {
            containers.insert(id.clone(), millicores(used));
        }
    }

    Some(Busy {
        node: millicores(after.busy.checked_sub(before.busy)?),
        capacity: cores() * 1000,
        containers,
    })
}

/// How many cores the machine has, for the figure the node's own is shown
/// against.
///
/// `available_parallelism` rather than counting `/proc/stat` lines: it
/// respects an affinity mask and a cgroup quota, so a node confined to two
/// cores of a big machine reports two — which is the number its own limits
/// make true.
fn cores() -> u32 {
    std::thread::available_parallelism()
        .map(|count| count.get() as u32)
        .unwrap_or(1)
}

/// Busy and total microseconds for the whole machine.
///
/// From `/proc/stat`'s first line, whose fields are jiffies — and here the
/// conversion **matters**, where an earlier version could shrug because
/// both sides of a ratio were in the same unit. Millicores are CPU time
/// over wall time, so a jiffy has to become a real duration.
///
/// `USER_HZ` is 100 for `/proc/stat` regardless of the kernel's internal
/// tick: the value there is scaled to `USER_HZ` by the kernel precisely so
/// userspace can rely on it, and `sysconf(_SC_CLK_TCK)` has answered 100 on
/// Linux for as long as anybody has shipped against it. The container
/// figures do not depend on this at all — a cgroup counts microseconds.
fn machine() -> (u64, u64) {
    let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    let Some(line) = stat.lines().next().filter(|line| line.starts_with("cpu ")) else {
        return (0, 0);
    };
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|field| field.parse().ok())
        .collect();
    let total: u64 = fields.iter().sum();
    // Field 3 is idle and field 4 is iowait: waiting on a disk is not the
    // CPU being busy, and counting it would make a node look loaded while
    // it sat still.
    let idle: u64 = fields.iter().skip(3).take(2).sum();
    const USEC_PER_JIFFY: u64 = 10_000;
    (
        total.saturating_sub(idle) * USEC_PER_JIFFY,
        total * USEC_PER_JIFFY,
    )
}

/// A cgroup's CPU time, in microseconds.
fn usage_usec(cgroup: &str) -> Option<u64> {
    let text = std::fs::read_to_string(format!("/sys/fs/cgroup{cgroup}/cpu.stat")).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("usage_usec "))
        .and_then(|value| value.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_of(at: Instant, busy: u64, total: u64, container: u64) -> Sample {
        Sample {
            at,
            busy,
            total,
            containers: BTreeMap::from([("demo.web".to_string(), container)]),
        }
    }

    /// The point of the module: a percentage is the difference between two
    /// readings, over the time the machine had in between.
    #[test]
    fn two_readings_make_a_rate() {
        let first = Instant::now();
        let before = sample_of(first, 0, 0, 0);
        // The machine had 1,000,000 µs and was busy for half of it; the
        // container used a tenth.
        let after = sample_of(
            first + std::time::Duration::from_secs(1),
            500_000,
            1_000_000,
            100_000,
        );

        let busy = between(&before, &after).expect("a rate");
        // Half a second of CPU in one second of wall clock is half a core.
        assert_eq!(busy.node, 500, "the machine");
        assert_eq!(busy.containers["demo.web"], 100, "and a tenth of one");
        assert!(busy.capacity >= 1000, "at least one core exists");
    }

    /// Millicores are absolute, which is the reason for choosing them: the
    /// same work reads the same on any machine, so two nodes' figures can
    /// be put beside each other. A percentage cannot — it would divide by
    /// a different core count on each.
    #[test]
    fn the_figure_does_not_depend_on_how_big_the_machine_is() {
        let first = Instant::now();
        let second = first + std::time::Duration::from_secs(1);

        // One core's worth of work, on a machine that had eight cores of
        // time to give and on one that had one.
        let small = between(
            &sample_of(first, 0, 0, 0),
            &sample_of(second, 1_000_000, 1_000_000, 1_000_000),
        )
        .expect("a rate");
        let large = between(
            &sample_of(first, 0, 0, 0),
            &sample_of(second, 1_000_000, 8_000_000, 1_000_000),
        )
        .expect("a rate");

        assert_eq!(small.containers["demo.web"], 1000);
        assert_eq!(
            large.containers["demo.web"], 1000,
            "the same work, said the same way"
        );
    }

    /// A counter that went backwards is a machine that rebooted or a
    /// container that was recreated. Neither is a negative percentage, and
    /// neither is a huge one.
    #[test]
    fn a_counter_that_went_backwards_says_nothing() {
        let first = Instant::now();
        let before = sample_of(first, 900_000, 1_000_000, 500_000);
        let after = sample_of(
            first + std::time::Duration::from_secs(1),
            10_000,
            20_000,
            1_000,
        );

        assert!(between(&before, &after).is_none());
    }

    /// A container in the second reading and not the first has just
    /// started. Counting all of its time as this interval's would show it
    /// at several hundred per cent for one tick.
    #[test]
    fn a_container_that_has_just_started_has_no_rate_yet() {
        let first = Instant::now();
        let mut before = sample_of(first, 0, 0, 0);
        before.containers.clear();
        let after = sample_of(
            first + std::time::Duration::from_secs(1),
            500_000,
            1_000_000,
            400_000,
        );

        let busy = between(&before, &after).expect("a rate");
        assert!(busy.containers.is_empty(), "{:?}", busy.containers);
        assert_eq!(busy.node, 500);
    }
}
