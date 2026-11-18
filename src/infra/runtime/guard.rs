//! The cgroup every container runs inside, and what it is allowed.
//!
//! ## Why a parent, when every service can have a ceiling
//!
//! A ceiling per service protects services from each other; it cannot
//! protect the node. A service without one may take everything, and
//! ceilings that each fit can still add up to more than the machine once
//! they are on different pages. The kernel enforces a limit on a parent
//! against the sum of everything under it, so with every container here
//! and `memory.max` at what the node may promise, the node keeps its
//! reserve whatever any service does — and an out-of-memory kill happens
//! inside this group, never to the console.
//!
//! Measured on node-0 before this existed: every container process had
//! `oom_score_adj` -998, inherited from containerd's unit, and
//! `wabot-deploy` had 0. The kernel's choice in a machine-wide shortage
//! was the console, the edge and every route with it.
//!
//! ## CPU is shared, memory is not
//!
//! Decided by Jorge on 2026-11-18: memory is never promised beyond what
//! the node may allocate, and CPU may be. So the parent's `cpu.max` is
//! what the node does not keep — services together never get the
//! reserve, even when the node is idle — and its weight is low, so under
//! contention below that line the node is served first.

use std::path::{Path, PathBuf};

use crate::domain::policies::presets;
use crate::infra::bootstrap::init::Init;

/// Written on every container, so a boot can tell one created before the
/// guard existed and recreate it inside. Its value names the parent and
/// not the limits: those are rewritten on the parent while the containers
/// run, and a value that carried them would recreate every service each
/// time the reserve moved.
pub const LABEL: &str = "wabot.guard";

const SLICE: &str = "wabot.slice";
const UNIT_FILE: &str = "/etc/systemd/system/wabot.slice";
const CGROUPFS_NAME: &str = "wabot";
const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// The node's processes default to 100, so under contention a service
/// gets a tenth of what the node gets.
const CPU_WEIGHT: u32 = 10;
const CPU_PERIOD: u64 = 100_000;

/// Below every container's, so in a machine-wide shortage the kernel takes
/// a service first. Not -1000, which would make the node unkillable even
/// when it is the one leaking.
const NODE_OOM_SCORE_ADJ: i32 = -900;

const _: () = assert!(
    super::spec::OOM_SCORE_ADJ > NODE_OOM_SCORE_ADJ,
    "a container must rank above the node for the OOM killer"
);

const CONTROLLERS: &str = "+memory +cpu +pids";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    Systemd,
    Cgroupfs,
}

impl Driver {
    /// The same answer [`super::client::RuncOptions::crun`] gives crun,
    /// because the path below is written in that driver's syntax.
    pub fn detect() -> Self {
        match Init::detect() {
            Init::Systemd => Self::Systemd,
            _ => Self::Cgroupfs,
        }
    }

    /// systemd's driver reads `slice:prefix:name`; cgroupfs reads a path.
    pub fn cgroups_path(self, container: &str) -> String {
        match self {
            Self::Systemd => format!("{SLICE}:crun:{container}"),
            Self::Cgroupfs => format!("/{CGROUPFS_NAME}/{container}"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Systemd => SLICE,
            Self::Cgroupfs => CGROUPFS_NAME,
        }
    }

    fn relative(self) -> String {
        format!("/{}", self.label())
    }

    fn directory(self) -> PathBuf {
        Path::new(CGROUP_ROOT).join(self.label())
    }

    /// Whether a `/proc/<pid>/cgroup` path is under the guard.
    pub fn holds(self, path: &str) -> bool {
        path.strip_prefix(&self.relative())
            .is_some_and(|rest| rest.starts_with('/'))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Limits {
    /// `None` when the machine's size could not be read: writing a number
    /// made of a failed reading would kill every container.
    pub memory: Option<u64>,
    pub cpu_millicores: Option<u32>,
    pub tasks: Option<u64>,
}

impl Limits {
    /// The same arithmetic the node refuses a promise with, so what the
    /// kernel enforces and what the form says fits cannot disagree.
    ///
    /// Tasks are three quarters of what the kernel can hold: a fork bomb in
    /// one container is stopped by its own `pids.max`, and this is for
    /// many containers together leaving the node unable to start a process.
    pub fn of(
        total_memory: u64,
        memory_reserve: Option<u64>,
        cores: u32,
        cpu_reserve: Option<u32>,
        threads: Option<u64>,
    ) -> Self {
        Self {
            memory: Some(presets::allocatable_memory(total_memory, memory_reserve))
                .filter(|bytes| total_memory > 0 && *bytes > 0),
            cpu_millicores: Some(presets::allocatable_cpu(
                cores.saturating_mul(1_000),
                cpu_reserve,
            ))
            .filter(|millicores| *millicores > 0),
            tasks: threads
                .map(|threads| threads * 3 / 4)
                .filter(|tasks| *tasks > 0),
        }
    }
}

/// How many tasks the kernel can hold: the smaller of its two ceilings.
pub fn threads() -> Option<u64> {
    let read =
        |path: &str| -> Option<u64> { std::fs::read_to_string(path).ok()?.trim().parse().ok() };
    match (
        read("/proc/sys/kernel/pid_max"),
        read("/proc/sys/kernel/threads-max"),
    ) {
        (Some(pids), Some(threads)) => Some(pids.min(threads)),
        (one, other) => one.or(other),
    }
}

/// Lower this process's standing with the OOM killer.
///
/// From the process rather than the unit or the OpenRC script, because
/// an update replaces the binary and never rewrites either.
pub fn protect_this_process() -> Result<(), String> {
    if cfg!(test) {
        return Err("a test does not change how the kernel ranks its process".into());
    }
    std::fs::write("/proc/self/oom_score_adj", NODE_OOM_SCORE_ADJ.to_string())
        .map_err(|error| format!("could not lower this process's OOM score: {error}"))
}

/// Make the parent exist with these limits. Convergent: it asks the
/// files, not whether it ran before.
pub async fn converge(limits: Limits) -> Result<String, String> {
    if cfg!(test) {
        return Err("a test does not change the machine's cgroups".into());
    }
    match Driver::detect() {
        Driver::Systemd => converge_systemd(limits).await,
        Driver::Cgroupfs => converge_cgroupfs(limits),
    }?;
    Ok(describe(&limits))
}

/// The file is what holds at boot; `set-property --runtime` is what holds
/// now. Both, because a reload is not documented to reapply cgroup
/// settings to a slice that is already running, and a runtime drop-in
/// outranks the file — so it is rewritten every time, never left stale.
///
/// Never stopped or restarted: that would stop every container in it.
async fn converge_systemd(limits: Limits) -> Result<(), String> {
    let unit = unit_file(&limits);
    if std::fs::read_to_string(UNIT_FILE).ok().as_deref() != Some(unit.as_str()) {
        std::fs::write(UNIT_FILE, &unit)
            .map_err(|error| format!("could not write {UNIT_FILE}: {error}"))?;
        systemctl(&["daemon-reload"]).await?;
    }
    systemctl(&["start", SLICE]).await?;
    let properties = properties(&limits);
    let mut arguments = vec!["set-property", "--runtime", SLICE];
    arguments.extend(properties.iter().map(String::as_str));
    systemctl(&arguments).await
}

async fn systemctl(arguments: &[&str]) -> Result<(), String> {
    let output = tokio::process::Command::new("systemctl")
        .args(arguments)
        .output()
        .await
        .map_err(|error| format!("could not run systemctl: {error}"))?;
    match output.status.success() {
        true => Ok(()),
        false => Err(format!(
            "systemctl {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

/// The controllers are enabled on the way down, because a cgroup only
/// offers its children what its own `subtree_control` lists — and crun
/// asking for a pids limit under a parent that offers no pids controller
/// fails the container.
fn converge_cgroupfs(limits: Limits) -> Result<(), String> {
    let root = Path::new(CGROUP_ROOT);
    let directory = Driver::Cgroupfs.directory();
    let write = |path: &Path, value: &str| {
        std::fs::write(path, value)
            .map_err(|error| format!("could not write {value:?} to {}: {error}", path.display()))
    };
    write(&root.join("cgroup.subtree_control"), CONTROLLERS)?;
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    for (file, value) in files(&limits) {
        let path = directory.join(file);
        // Present only where the kernel accounts swap. A machine without it
        // has none to give, which is what writing 0 would have said.
        if file == "memory.swap.max" && !path.exists() {
            continue;
        }
        write(&path, &value)?;
    }
    write(&directory.join("cgroup.subtree_control"), CONTROLLERS)
}

fn unit_file(limits: &Limits) -> String {
    let mut unit = String::from(
        "# Written by wabot-deploy on every start. Every container it runs is under\n\
         # this slice, and these limits are what keeps the rest of the machine for\n\
         # the node. Edits are overwritten; the reserves are on the node's page.\n\
         [Unit]\n\
         Description=Services run by wabot-deploy\n\
         \n\
         [Slice]\n",
    );
    for property in properties(limits) {
        // An empty assignment resets a property on the command line and
        // means nothing in a file.
        if !property.ends_with('=') {
            unit.push_str(&property);
            unit.push('\n');
        }
    }
    unit
}

fn properties(limits: &Limits) -> Vec<String> {
    vec![
        "MemoryAccounting=yes".into(),
        format!(
            "MemoryMax={}",
            limits
                .memory
                .map_or("infinity".to_string(), |bytes| bytes.to_string())
        ),
        "MemorySwapMax=0".into(),
        "CPUAccounting=yes".into(),
        format!("CPUWeight={CPU_WEIGHT}"),
        // A percentage of one core, in whole points: 1750 millicores is
        // 175 %. Rounded down, so the line is never above what was asked.
        format!(
            "CPUQuota={}",
            limits
                .cpu_millicores
                .map_or(String::new(), |millicores| format!(
                    "{}%",
                    (millicores / 10).max(1)
                ))
        ),
        "TasksAccounting=yes".into(),
        format!(
            "TasksMax={}",
            limits
                .tasks
                .map_or("infinity".to_string(), |tasks| tasks.to_string())
        ),
    ]
}

fn files(limits: &Limits) -> Vec<(&'static str, String)> {
    vec![
        (
            "memory.max",
            limits
                .memory
                .map_or("max".to_string(), |bytes| bytes.to_string()),
        ),
        ("memory.swap.max", "0".into()),
        ("cpu.weight", CPU_WEIGHT.to_string()),
        (
            "cpu.max",
            match limits.cpu_millicores {
                Some(millicores) => format!(
                    "{} {CPU_PERIOD}",
                    u64::from(millicores) * CPU_PERIOD / 1_000
                ),
                None => format!("max {CPU_PERIOD}"),
            },
        ),
        (
            "pids.max",
            limits
                .tasks
                .map_or("max".to_string(), |tasks| tasks.to_string()),
        ),
    ]
}

fn describe(limits: &Limits) -> String {
    format!(
        "services together: memory {}, CPU {}, tasks {}",
        limits
            .memory
            .map_or("unbounded".to_string(), crate::infra::node::memory::human),
        limits
            .cpu_millicores
            .map_or("unbounded".to_string(), presets::cpu_label),
        limits
            .tasks
            .map_or("unbounded".to_string(), |tasks| tasks.to_string()),
    )
}

/// `doctor`'s line, untranslated like the rest of it.
pub fn summary(reading: Option<&Reading>) -> String {
    let Some(reading) = reading else {
        return format!(
            "absent — services are not held apart from the node; `serve` makes {} at start",
            Driver::detect().label()
        );
    };
    format!(
        "{} — memory {} (using {}), CPU {}, tasks {} (using {}), {} killed for memory",
        Driver::detect().label(),
        reading
            .limits
            .memory
            .map_or("UNBOUNDED".to_string(), crate::infra::node::memory::human),
        crate::infra::node::memory::human(reading.memory_used),
        reading
            .limits
            .cpu_millicores
            .map_or("unbounded".to_string(), presets::cpu_label),
        reading
            .limits
            .tasks
            .map_or("unbounded".to_string(), |tasks| tasks.to_string()),
        reading.tasks_used,
        reading.processes_killed,
    )
}

/// What the kernel says the parent is, which is what a page and `doctor`
/// show — not what this process last asked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reading {
    pub limits: Limits,
    pub memory_used: u64,
    pub tasks_used: u64,
    pub processes_killed: u64,
}

pub fn read() -> Option<Reading> {
    let directory = Driver::detect().directory();
    let file = |name: &str| std::fs::read_to_string(directory.join(name)).ok();
    let number = |name: &str| file(name).and_then(|text| parse_max(&text));
    let memory_max = file("memory.max")?;
    Some(Reading {
        limits: Limits {
            memory: parse_max(&memory_max),
            cpu_millicores: file("cpu.max").and_then(|text| parse_cpu_max(&text)),
            tasks: number("pids.max"),
        },
        memory_used: number("memory.current").unwrap_or(0),
        tasks_used: number("pids.current").unwrap_or(0),
        processes_killed: file("memory.events")
            .and_then(|events| {
                events.lines().find_map(|line| {
                    line.strip_prefix("oom_kill ")
                        .and_then(|count| count.trim().parse().ok())
                })
            })
            .unwrap_or(0),
    })
}

fn parse_max(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

fn parse_cpu_max(text: &str) -> Option<u32> {
    let mut words = text.split_whitespace();
    let quota: u64 = words.next()?.parse().ok()?;
    let period: u64 = words.next()?.parse().ok().filter(|period| *period > 0)?;
    u32::try_from(quota * 1_000 / period).ok()
}

/// Whether this process is under the guard, from where the kernel says it
/// is rather than from the label it was created with.
pub fn holds_pid(pid: u32) -> bool {
    let Ok(cgroup) = std::fs::read_to_string(format!("/proc/{pid}/cgroup")) else {
        return false;
    };
    cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .is_some_and(|path| Driver::detect().holds(path.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn the_guard_holds_what_the_node_may_promise_and_no_more() {
        let limits = Limits::of(2048 * MB, Some(512 * MB), 2, Some(250), Some(15_000));
        assert_eq!(limits.memory, Some(1536 * MB));
        assert_eq!(limits.cpu_millicores, Some(1_750));
        assert_eq!(limits.tasks, Some(11_250));

        assert_eq!(
            limits.memory,
            Some(presets::allocatable_memory(2048 * MB, Some(512 * MB))),
            "the kernel's number is the one the forms refuse with"
        );
    }

    /// A machine whose size could not be read must not be written as a
    /// machine with nothing: `memory.max 0` kills every container in it.
    #[test]
    fn an_unread_machine_is_left_unbounded_rather_than_emptied() {
        let limits = Limits::of(0, None, 0, None, None);
        assert_eq!(limits, Limits::default());
        assert_eq!(files(&limits)[0], ("memory.max", "max".to_string()));
        assert!(properties(&limits).contains(&"MemoryMax=infinity".to_string()));
    }

    #[test]
    fn the_two_drivers_name_the_same_parent_in_their_own_syntax() {
        assert_eq!(
            Driver::Systemd.cgroups_path("acme.web"),
            "wabot.slice:crun:acme.web"
        );
        assert_eq!(Driver::Cgroupfs.cgroups_path("acme.web"), "/wabot/acme.web");

        assert!(Driver::Systemd.holds("/wabot.slice/crun-acme.web.scope/container"));
        assert!(Driver::Cgroupfs.holds("/wabot/acme.web"));
        assert!(!Driver::Systemd.holds("/system.slice/crun-acme.web.scope/container"));
        assert!(
            !Driver::Cgroupfs.holds("/wabotics/acme.web"),
            "a sibling whose name starts the same is not inside"
        );
        assert!(
            !Driver::Cgroupfs.holds("/wabot"),
            "the parent holds no process itself"
        );
    }

    #[test]
    fn a_cpu_ceiling_reads_back_as_the_millicores_it_was_written_from() {
        let limits = Limits {
            cpu_millicores: Some(1_750),
            ..Default::default()
        };
        let (_, written) = files(&limits)
            .into_iter()
            .find(|(file, _)| *file == "cpu.max")
            .expect("cpu.max");
        assert_eq!(written, "175000 100000");
        assert_eq!(parse_cpu_max(&written), Some(1_750));
        assert_eq!(parse_cpu_max("max 100000"), None);
        assert!(properties(&limits).contains(&"CPUQuota=175%".to_string()));
    }

    /// `CPUQuota=` resets the quota on the command line and is an error in
    /// a unit file, so the file leaves out what has no value.
    #[test]
    fn the_unit_file_carries_only_the_properties_that_have_a_value() {
        let unbounded_cpu = Limits {
            memory: Some(512 * MB),
            cpu_millicores: None,
            tasks: Some(1_000),
        };
        let unit = unit_file(&unbounded_cpu);
        assert!(unit.contains("MemoryMax=536870912\n"), "{unit}");
        assert!(unit.contains("MemorySwapMax=0\n"), "{unit}");
        assert!(unit.contains("CPUWeight=10\n"), "{unit}");
        assert!(!unit.contains("CPUQuota"), "{unit}");
        assert!(properties(&unbounded_cpu).contains(&"CPUQuota=".to_string()));
    }

    #[test]
    fn the_parent_says_what_the_kernel_counted() {
        assert_eq!(parse_max("max\n"), None);
        assert_eq!(parse_max("1610612736\n"), Some(1_610_612_736));
    }
}
