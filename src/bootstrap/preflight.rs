//! Is this machine one the node can run on?
//!
//! Every check here answers a question that, unanswered, becomes a
//! confusing failure later: a container that will not start because
//! the kernel has cgroup v1, an OOM kill nobody can explain because
//! there is no swap and no memory accounting, a bind that fails
//! because something else already owns port 443.
//!
//! ## Blocking and advisory
//!
//! A [`Severity::Blocking`] failure stops the install: proceeding
//! would produce a node that cannot work. A [`Severity::Advisory`] one
//! is printed and stepped over — the node will run, someone should
//! know. Getting this split wrong in either direction is its own
//! failure: a blocking check that should be advisory makes the product
//! refuse machines it could serve, and the reverse ships a node that
//! breaks on first use.

use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The node cannot work here.
    Blocking,
    /// The node will work; something is worth knowing.
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail(Severity),
    /// Could not be determined — an unreadable `/proc`, an unusual
    /// distribution. Reported, never blocking: refusing to install
    /// because a check could not run is worse than installing and
    /// finding out.
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Check {
    pub name: &'static str,
    pub outcome: Outcome,
    pub detail: String,
}

impl Check {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            outcome: Outcome::Pass,
            detail: detail.into(),
        }
    }

    fn blocking(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            outcome: Outcome::Fail(Severity::Blocking),
            detail: detail.into(),
        }
    }

    fn advisory(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            outcome: Outcome::Fail(Severity::Advisory),
            detail: detail.into(),
        }
    }

    fn unknown(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            outcome: Outcome::Unknown,
            detail: detail.into(),
        }
    }

    pub fn blocks(&self) -> bool {
        self.outcome == Outcome::Fail(Severity::Blocking)
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mark = match self.outcome {
            Outcome::Pass => "ok  ",
            Outcome::Fail(Severity::Blocking) => "STOP",
            Outcome::Fail(Severity::Advisory) => "warn",
            Outcome::Unknown => "?   ",
        };
        write!(f, "{mark}  {:<14} {}", self.name, self.detail)
    }
}

/// What the node needs from the machine.
///
/// `ports` is separate from the rest because `doctor` wants the same
/// checks without complaining that the running node holds its own
/// ports.
pub fn run(https_port: u16, http_port: u16, check_ports: bool) -> Vec<Check> {
    let mut checks = vec![
        operating_system(),
        privileges(),
        architecture(),
        init_system(),
        cgroup_v2(),
        overlayfs(),
        memory(),
    ];
    if check_ports {
        checks.push(port_free("port 443", https_port));
        checks.push(port_free("port 80", http_port));
    }
    checks
}

fn operating_system() -> Check {
    if cfg!(target_os = "linux") {
        Check::pass("os", "Linux")
    } else {
        Check::blocking(
            "os",
            format!(
                "{} — containerd, cgroups and systemd are Linux. Build for a Linux target.",
                std::env::consts::OS
            ),
        )
    }
}

fn privileges() -> Check {
    // Reading the real uid rather than shelling out to `id`: one
    // fewer process and no dependency on a coreutils layout.
    #[cfg(unix)]
    {
        let uid = unsafe { libc_getuid() };
        if uid == 0 {
            Check::pass("privileges", "root")
        } else {
            Check::blocking(
                "privileges",
                format!(
                    "uid {uid} — installing writes /etc, /usr/local/bin and a systemd unit, \
                     and the node binds ports below 1024. Re-run with sudo."
                ),
            )
        }
    }
    #[cfg(not(unix))]
    Check::unknown("privileges", "not a unix host")
}

#[cfg(unix)]
unsafe fn libc_getuid() -> u32 {
    // The one libc call worth making directly rather than adding a
    // dependency for.
    extern "C" {
        fn getuid() -> u32;
    }
    getuid()
}

fn architecture() -> Check {
    match std::env::consts::ARCH {
        arch @ ("x86_64" | "aarch64") => Check::pass("architecture", arch),
        other => Check::advisory(
            "architecture",
            format!(
                "{other} — containerd and crun release binaries cover x86_64 and aarch64; \
                     you will need to supply your own"
            ),
        ),
    }
}

fn init_system() -> Check {
    use crate::bootstrap::init::Init;
    match Init::detect() {
        Init::Systemd => Check::pass("init", "systemd"),
        Init::OpenRc => Check::pass("init", "OpenRC"),
        Init::None => Check::advisory(
            "init",
            "no service manager found — the node runs fine in the foreground, but \
             `install` cannot register a service and nothing will restart it",
        ),
    }
}

/// cgroup v2, unified.
///
/// Not a preference. Memory limits and OOM accounting behave
/// differently under v1, and the autosizing this platform inherits
/// depends on reading an OOM correctly. A node on v1 would deploy
/// containers and mis-report why they died.
fn cgroup_v2() -> Check {
    const UNIFIED: &str = "/sys/fs/cgroup/cgroup.controllers";
    if Path::new(UNIFIED).exists() {
        Check::pass("cgroups", "v2 unified")
    } else if Path::new("/sys/fs/cgroup/memory").exists() {
        Check::blocking(
            "cgroups",
            "v1 — memory limits and OOM accounting differ, and this node reads both. \
             Boot with systemd.unified_cgroup_hierarchy=1.",
        )
    } else if matches!(
        crate::bootstrap::init::Init::detect(),
        crate::bootstrap::init::Init::OpenRc
    ) {
        // Alpine mounts no hierarchy until its `cgroups` service runs.
        // `install` turns it on, so this is a note about what is about
        // to happen rather than something to go and do.
        Check::advisory(
            "cgroups",
            "nothing mounted yet — `install` enables OpenRC's cgroups service, which is \
             what mounts the v2 hierarchy containerd needs",
        )
    } else {
        Check::unknown("cgroups", "could not read /sys/fs/cgroup")
    }
}

fn overlayfs() -> Check {
    let Ok(filesystems) = std::fs::read_to_string("/proc/filesystems") else {
        return Check::unknown("overlayfs", "could not read /proc/filesystems");
    };
    if filesystems.lines().any(|line| line.ends_with("overlay")) {
        Check::pass("overlayfs", "available")
    } else {
        Check::advisory(
            "overlayfs",
            "not in /proc/filesystems — `install` tries `modprobe overlay`; without it \
             containerd falls back to a slower snapshotter",
        )
    }
}

/// Memory, and the swap that decides whether a shortfall is slow or
/// fatal.
fn memory() -> Check {
    let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") else {
        return Check::unknown("memory", "could not read /proc/meminfo");
    };

    let field = |name: &str| -> Option<u64> {
        meminfo
            .lines()
            .find(|line| line.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };

    let total_mb = field("MemTotal:").unwrap_or(0) / 1024;
    let swap_mb = field("SwapTotal:").unwrap_or(0) / 1024;

    // The node itself is ~12 MB and containerd ~50 MB. Under 512 MB
    // total there is nothing left for the containers that are the
    // point of the machine.
    if total_mb < 512 {
        Check::advisory(
            "memory",
            format!(
                "{total_mb} MB and {swap_mb} MB swap — the control plane needs ~70 MB, \
                     leaving very little for containers"
            ),
        )
    } else if swap_mb == 0 && total_mb < 2048 {
        Check::advisory(
            "memory",
            format!(
                "{total_mb} MB, no swap — a memory spike will be an OOM kill rather \
                     than a slow moment"
            ),
        )
    } else {
        Check::pass("memory", format!("{total_mb} MB, {swap_mb} MB swap"))
    }
}

/// Is the port free?
///
/// Bound and released rather than parsed out of `ss`: what matters is
/// whether a process can bind it, which is the question the node will
/// ask a minute later.
///
/// Retried once, because a bind can fail for a beat after the previous
/// owner closed — which is exactly the case of restarting the node.
/// The real listener is tokio's, which sets `SO_REUSEADDR` and would
/// have succeeded; without the retry this check is stricter than
/// reality and would refuse an install that would have worked. A
/// socket someone is actually listening on fails both times.
fn port_free(name: &'static str, port: u16) -> Check {
    let mut last = match try_bind(port) {
        Ok(()) => return Check::pass(name, "free"),
        Err(error) => error,
    };

    if last.kind() != std::io::ErrorKind::PermissionDenied {
        std::thread::sleep(std::time::Duration::from_millis(150));
        match try_bind(port) {
            Ok(()) => return Check::pass(name, "free"),
            Err(error) => last = error,
        }
    }

    if last.kind() == std::io::ErrorKind::PermissionDenied {
        Check::blocking(
            name,
            format!("permission denied — binding {port} needs root or CAP_NET_BIND_SERVICE"),
        )
    } else {
        Check::blocking(
            name,
            format!("{last} — something already listens there. `ss -tlnp | grep :{port}`"),
        )
    }
}

fn try_bind(port: u16) -> std::io::Result<()> {
    std::net::TcpListener::bind(("0.0.0.0", port)).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list is the plan, and a check that reports nothing is one
    /// nobody will notice is broken.
    #[test]
    fn every_check_reports_something() {
        for check in run(443, 80, false) {
            assert!(!check.name.is_empty());
            assert!(
                !check.detail.is_empty(),
                "{} says nothing about why",
                check.name
            );
        }
    }

    #[test]
    fn check_names_are_unique() {
        let checks = run(443, 80, true);
        let mut names: Vec<&str> = checks.iter().map(|check| check.name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "two checks share a name");
    }

    /// Ports are the one check `doctor` has to be able to skip: the
    /// running node holds them, and reporting that as a problem would
    /// make a healthy node look broken.
    #[test]
    fn port_checks_are_optional() {
        assert!(run(443, 80, false)
            .iter()
            .all(|check| !check.name.starts_with("port")));
        assert_eq!(
            run(443, 80, true)
                .iter()
                .filter(|check| check.name.starts_with("port"))
                .count(),
            2
        );
    }

    /// A free port passes and a taken one blocks — the two answers the
    /// check exists to distinguish.
    #[test]
    fn a_taken_port_is_reported() {
        let listener = std::net::TcpListener::bind("0.0.0.0:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let taken = port_free("port test", port);
        assert!(taken.blocks(), "a held port blocks: {taken}");
        assert!(taken.detail.contains(&port.to_string()), "and names it");

        drop(listener);
        assert_eq!(
            port_free("port test", port).outcome,
            Outcome::Pass,
            "a released port reads as free — the retry is what makes this reliable, \
             and without it a node restart would fail its own preflight"
        );
    }

    /// Whatever this machine is, the checks have to run on it without
    /// panicking — a preflight that crashes is worse than none.
    #[test]
    fn the_checks_run_here() {
        let checks = run(0, 0, false);
        assert!(checks.len() >= 6);

        // Which checks block depends on where this runs — a Mac
        // fails `os`, an unprivileged shell fails `privileges` — so
        // the assertion is about the *quality* of a refusal rather
        // than a list that changes with the runner.
        for check in checks.iter().filter(|check| check.blocks()) {
            assert!(
                check.detail.len() > 20,
                "{} blocks the install but does not say what to do: {:?}",
                check.name,
                check.detail
            );
        }

        // The OS check is the one whose answer is known at compile
        // time, so it can be asserted exactly.
        let os = checks.iter().find(|check| check.name == "os").unwrap();
        assert_eq!(os.blocks(), !cfg!(target_os = "linux"));
    }

    #[test]
    fn display_marks_severity() {
        assert!(Check::pass("x", "fine").to_string().starts_with("ok"));
        assert!(Check::blocking("x", "no").to_string().starts_with("STOP"));
        assert!(Check::advisory("x", "hm").to_string().starts_with("warn"));
        assert!(Check::unknown("x", "?").to_string().starts_with("?"));
    }
}
