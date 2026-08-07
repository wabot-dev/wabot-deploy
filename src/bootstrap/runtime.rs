//! Getting containerd and crun onto the machine.
//!
//! ## Why not the distribution's package
//!
//! Every release of Ubuntu, Debian and RHEL ships a different
//! containerd version with a different default configuration, and that
//! divergence is paid for in support rather than saved in effort. A
//! pinned tarball is the version we tested, on every distribution.
//!
//! It is also barely a choice: the containerd tarball contains no
//! runtime, and what the distributions package is runc, not crun. A
//! binary has to be fetched either way.
//!
//! ## Why crun
//!
//! C rather than Go: a 300 KB binary against runc's 15 MB, and
//! 15–25% faster container startup. What it does *not* change is the
//! resident cost per container — that is the shim, one per container
//! at ~11 MB, and the shim is Go whichever runtime it drives. Worth
//! knowing before anyone optimises the wrong number.
//!
//! ## The configuration does not select crun for *our* path
//!
//! `config.toml` sets `BinaryName` under the CRI plugin, which is the
//! Kubernetes-facing API this node does not use. The native API takes
//! the runtime per container, in `Containers.Create`. Verified rather
//! than assumed: with only the configuration set, `ctr run` still
//! looked for runc and failed. The section stays anyway, so that the
//! tools someone reaches for on a broken node use crun too.
//!
//! ## An existing containerd is left alone
//!
//! If one is already running — Docker installed, most likely — the
//! node uses its socket and touches nothing. Rewriting somebody
//! else's daemon configuration to suit us is not a thing an installer
//! should do.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::init::{Init, ServiceFile};

/// Pinned, not "latest": the version this was tested against, on every
/// distribution, until someone raises it deliberately.
pub const CONTAINERD_VERSION: &str = "2.3.3";
pub const CRUN_VERSION: &str = "1.28";
pub const CNI_VERSION: &str = "1.9.1";

/// The containerd socket. The default path, which is what a
/// pre-existing installation will be using too.
pub const SOCKET: &str = "/run/containerd/containerd.sock";

/// Where crun lands. `/usr/local/bin` rather than `/usr/bin`: it is
/// not the distribution's, and a package manager should never find it
/// in a directory it owns.
pub const CRUN_PATH: &str = "/usr/local/bin/crun";

/// Where the CNI plugins land. Not a choice: `/opt/cni/bin` is the
/// path the whole ecosystem defaults to, and a container runtime
/// looking for them looks there.
pub const CNI_BIN_DIR: &str = "/opt/cni/bin";

/// The plugins a project network actually needs.
///
/// `bridge` builds the network, `host-local` hands out addresses in
/// it, `loopback` brings up `lo` inside the namespace — a container
/// without it cannot talk to itself, which surprises everything that
/// binds `127.0.0.1`. `portmap` forwards a node port to one inside the
/// container, for a service published as raw TCP.
///
/// Listed by name rather than trusting the directory: a half-extracted
/// tarball leaves a `/opt/cni/bin` that exists and cannot build a
/// network.
pub const CNI_PLUGINS: &[&str] = &["bridge", "host-local", "loopback", "portmap"];

/// SHA-256 of each release artifact.
///
/// crun publishes GPG signatures but no checksum file, so these were
/// computed once from the published binaries and written down. That is
/// the same guarantee a checksum file gives — the bytes are the ones
/// we looked at — without a keyring to carry.
const CHECKSUMS: &[(&str, &str, &str)] = &[
    (
        "containerd",
        "x86_64",
        "34d418fbea898a7787edb869c17b7d3c56d3314d42226032dd6f0e62cfdd18a5",
    ),
    (
        "containerd",
        "aarch64",
        "2618fbdbf55e26897f03b4d01b78e66d6746865d19ca72469b2675c4a62e5322",
    ),
    (
        "crun",
        "x86_64",
        "2aa6b7024a9c9f153895c0d11ae233d3758f54844011c3a039e3e89048d01d42",
    ),
    (
        "crun",
        "aarch64",
        "cc1e8ec89aef1422e0741be196f9ed099e2e09d2f48f30f27cd44a22ef1f0342",
    ),
    // These two are the project's own published `.sha256` files, not
    // ones computed here — containernetworking publishes them beside
    // the tarballs, so the guarantee comes from upstream.
    (
        "cni-plugins",
        "x86_64",
        "b98f74a0f8522f0a83867178729c1aa70f2158f90c45a2ca8fa791db1c76b303",
    ),
    (
        "cni-plugins",
        "aarch64",
        "56171987d3947707c3563db2f4001bccaf50fd63468611b9f3cbecb1375ee7ec",
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("{0}")]
    Unsupported(String),
    #[error("{what} download failed: {detail}")]
    Download { what: &'static str, detail: String },
    #[error("{what}: expected sha256 {expected}, got {actual} — refusing to install it")]
    Checksum {
        what: &'static str,
        expected: String,
        actual: String,
    },
    #[error("{0}")]
    Command(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

type RuntimeResult<T> = Result<T, RuntimeError>;

/// What the machine already has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub containerd: Option<String>,
    pub crun: Option<String>,
    pub socket: bool,
    /// Every plugin in [`CNI_PLUGINS`] is present.
    pub cni: bool,
}

impl Status {
    pub fn ready(&self) -> bool {
        self.containerd.is_some() && self.crun.is_some() && self.socket && self.cni
    }
}

pub fn status() -> Status {
    Status {
        containerd: version_of("containerd", &["--version"]),
        crun: version_of(CRUN_PATH, &["--version"]).or_else(|| version_of("crun", &["--version"])),
        socket: Path::new(SOCKET).exists(),
        cni: cni_installed(),
    }
}

/// Are the plugins we invoke all there?
///
/// Checked by name rather than by directory: a half-extracted tarball
/// leaves a `/opt/cni/bin` that exists and cannot build a network.
fn cni_installed() -> bool {
    CNI_PLUGINS
        .iter()
        .all(|plugin| Path::new(CNI_BIN_DIR).join(plugin).exists())
}

/// First line of `<program> --version`, or `None` if it is not there.
fn version_of(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
}

/// Install what is missing and make sure containerd is running.
///
/// Idempotent: a machine that already has both is left as it is, and
/// the function reports that rather than reinstalling.
pub fn ensure() -> RuntimeResult<String> {
    let mut done: Vec<String> = Vec::new();

    if status().containerd.is_none() {
        install_containerd()?;
        done.push(format!("containerd {CONTAINERD_VERSION}"));
    }
    if status().crun.is_none() {
        install_crun()?;
        done.push(format!("crun {CRUN_VERSION}"));
    }
    if !cni_installed() {
        install_cni()?;
        done.push(format!("cni plugins {CNI_VERSION}"));
    }
    // Asked of the machine, not of this run: a node that was rebooted
    // without the sysctl file, or one where somebody turned it off,
    // needs it turned back on.
    if !forwarding_enabled() {
        enable_forwarding()?;
        done.push("ip forwarding".into());
    }

    // What a systemd distribution has already done for you. Alpine
    // mounts no cgroup hierarchy and loads no overlay module until
    // something asks, and containerd asks by failing.
    if Init::detect() == Init::OpenRc {
        if !cgroups_mounted() {
            enable_cgroups()?;
            done.push("cgroups".into());
        }
        if !overlay_available() && load_overlay() {
            done.push("overlay".into());
        }
    }

    // Each of these asks about the *thing*, not about whether this run
    // created it. An earlier version keyed the configuration off
    // "did we just install containerd", and a run that installed the
    // binaries and then failed to start them could never finish: the
    // next run saw containerd present, skipped the configuration it
    // had never written, and failed the same way forever. Resumability
    // is the point of the ledger, and this is where it is earned.
    if ours() && !Path::new(CONFIG_PATH).exists() {
        write_config()?;
        done.push("configuration".into());
    }
    if ours() && !containerd_service_path().exists() {
        write_unit()?;
        done.push("unit".into());
    }

    if !Path::new(SOCKET).exists() {
        start_containerd()?;
        done.push("started".into());
    }

    Ok(if done.is_empty() {
        "already present".to_string()
    } else {
        done.join(", ")
    })
}

/// Is this containerd the one we installed?
///
/// The discriminator for whether its configuration is ours to write.
/// Ours lives in `/usr/local/bin`; a distribution's is in `/usr/bin`,
/// and rewriting the configuration of a daemon somebody else installed
/// — Docker's, most likely — is not a thing an installer should do.
fn ours() -> bool {
    Path::new("/usr/local/bin/containerd").exists()
}

fn arch() -> RuntimeResult<&'static str> {
    match std::env::consts::ARCH {
        arch @ ("x86_64" | "aarch64") => Ok(arch),
        other => Err(RuntimeError::Unsupported(format!(
            "no containerd or crun release for {other}; install both by hand and re-run"
        ))),
    }
}

/// The name the release artifacts use, which is not the one Rust uses.
fn release_arch(arch: &str) -> &'static str {
    match arch {
        "aarch64" => "arm64",
        _ => "amd64",
    }
}

fn checksum_for(what: &str, arch: &str) -> RuntimeResult<&'static str> {
    CHECKSUMS
        .iter()
        .find(|(name, a, _)| *name == what && *a == arch)
        .map(|(_, _, sum)| *sum)
        .ok_or_else(|| {
            RuntimeError::Unsupported(format!("no pinned checksum for {what} on {arch}"))
        })
}

fn install_containerd() -> RuntimeResult<()> {
    let arch = arch()?;
    // The *static* build: no glibc dependency, so one artifact works
    // on Alpine and on RHEL alike.
    let url = format!(
        "https://github.com/containerd/containerd/releases/download/v{CONTAINERD_VERSION}\
         /containerd-static-{CONTAINERD_VERSION}-linux-{}.tar.gz",
        release_arch(arch)
    );

    let tarball = download("containerd", &url, checksum_for("containerd", arch)?)?;
    tracing::info!(version = CONTAINERD_VERSION, "installing containerd");

    // The tarball lays out bin/containerd, bin/ctr, bin/containerd-shim-runc-v2.
    run(
        "tar",
        &["-xzf", &tarball.to_string_lossy(), "-C", "/usr/local"],
    )?;
    let _ = std::fs::remove_file(&tarball);
    Ok(())
}

fn install_crun() -> RuntimeResult<()> {
    let arch = arch()?;
    let url = format!(
        "https://github.com/containers/crun/releases/download/{CRUN_VERSION}\
         /crun-{CRUN_VERSION}-linux-{}",
        release_arch(arch)
    );

    let binary = download("crun", &url, checksum_for("crun", arch)?)?;
    tracing::info!(version = CRUN_VERSION, "installing crun");

    std::fs::create_dir_all("/usr/local/bin")?;
    std::fs::copy(&binary, CRUN_PATH)?;
    let _ = std::fs::remove_file(&binary);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(CRUN_PATH, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn install_cni() -> RuntimeResult<()> {
    let arch = arch()?;
    let url = format!(
        "https://github.com/containernetworking/plugins/releases/download/v{CNI_VERSION}\
         /cni-plugins-linux-{}-v{CNI_VERSION}.tgz",
        release_arch(arch)
    );

    let tarball = download("cni-plugins", &url, checksum_for("cni-plugins", arch)?)?;
    tracing::info!(version = CNI_VERSION, "installing cni plugins");

    // The tarball is flat: the binaries sit at its root.
    std::fs::create_dir_all(CNI_BIN_DIR)?;
    run(
        "tar",
        &["-xzf", &tarball.to_string_lossy(), "-C", CNI_BIN_DIR],
    )?;
    let _ = std::fs::remove_file(&tarball);
    Ok(())
}

/// Where the sysctl lives, so it survives a reboot.
pub const SYSCTL_PATH: &str = "/etc/sysctl.d/99-wabot-deploy.conf";

/// Is there a cgroup hierarchy at all?
///
/// The unified controllers file is the honest probe: `/sys/fs/cgroup`
/// existing as a directory says nothing about anything being mounted
/// on it.
fn cgroups_mounted() -> bool {
    Path::new("/sys/fs/cgroup/cgroup.controllers").exists()
        || Path::new("/sys/fs/cgroup/memory").exists()
}

/// Turn OpenRC's cgroups service on, now and at boot.
///
/// containerd needs it — memory limits and OOM accounting are cgroup
/// facts — and this node's own containerd service declares `need
/// cgroups`, so without this the whole chain refuses to start with a
/// message about a dependency rather than about cgroups.
fn enable_cgroups() -> RuntimeResult<()> {
    run("rc-update", &["add", "cgroups", "boot"])?;
    run("rc-service", &["cgroups", "start"])?;
    Ok(())
}

fn overlay_available() -> bool {
    std::fs::read_to_string("/proc/filesystems")
        .map(|text| text.contains("overlay"))
        .unwrap_or(false)
}

/// Load overlayfs and keep it loaded across reboots.
///
/// Best effort, and `false` when it does not work: the kernel may have
/// it built in under another name, or refuse it entirely, and
/// containerd falls back to a slower snapshotter rather than failing.
/// Turning "your snapshotter will be slower" into "your install
/// failed" would be the wrong trade.
fn load_overlay() -> bool {
    if run("modprobe", &["overlay"]).is_err() {
        return false;
    }
    // `/etc/modules` is what Alpine's `modules` service reads at boot.
    let listed = std::fs::read_to_string("/etc/modules")
        .map(|text| text.lines().any(|line| line.trim() == "overlay"))
        .unwrap_or(false);
    if !listed {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/etc/modules")
        {
            let _ = writeln!(file, "overlay");
        }
    }
    overlay_available()
}

fn forwarding_enabled() -> bool {
    std::fs::read_to_string("/proc/sys/net/ipv4/ip_forward")
        .map(|value| value.trim() == "1")
        .unwrap_or(false)
}

/// Let packets cross between the project bridges and the outside.
///
/// Without it a container gets an address, reaches its own bridge, and
/// nothing else — which looks like a broken image rather than a
/// missing kernel flag. Written to `/etc/sysctl.d` as well as applied,
/// because a reboot would otherwise take the network away again.
fn enable_forwarding() -> RuntimeResult<()> {
    std::fs::create_dir_all("/etc/sysctl.d")?;
    std::fs::write(
        SYSCTL_PATH,
        "# Written by wabot-deploy. Containers live on per-project\n\
         # bridges, and reaching anything past them is forwarding.\n\
         net.ipv4.ip_forward = 1\n",
    )?;
    run("sysctl", &["-q", "-w", "net.ipv4.ip_forward=1"])?;
    Ok(())
}

/// Fetch to a temporary file and verify before it is used.
///
/// Verified *before* anything runs it, which is the whole point: a
/// truncated download and a substituted one look the same to `tar`.
fn download(what: &'static str, url: &str, expected: &str) -> RuntimeResult<PathBuf> {
    let destination = std::env::temp_dir().join(format!("wabot-deploy-{what}"));
    tracing::info!(%url, "downloading");

    // curl rather than an HTTP client in-process: it is on every
    // machine that can reach a package mirror, it handles the
    // redirect GitHub serves, and it keeps a TLS stack and a
    // resumable-download implementation out of this binary.
    let output = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "20",
            "--max-time",
            "600",
            "--output",
            &destination.to_string_lossy(),
            url,
        ])
        .output()
        .map_err(|error| RuntimeError::Download {
            what,
            detail: format!("could not run curl: {error}"),
        })?;

    if !output.status.success() {
        return Err(RuntimeError::Download {
            what,
            detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let actual = sha256_of(&destination)?;
    if actual != expected {
        let _ = std::fs::remove_file(&destination);
        return Err(RuntimeError::Checksum {
            what,
            expected: expected.to_string(),
            actual,
        });
    }
    Ok(destination)
}

fn sha256_of(path: &Path) -> RuntimeResult<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub const CONFIG_PATH: &str = "/etc/containerd/config.toml";

/// What containerd needs to be to serve this node.
fn write_config() -> RuntimeResult<()> {
    std::fs::create_dir_all("/etc/containerd")?;
    std::fs::write(CONFIG_PATH, CONTAINERD_CONFIG)?;
    Ok(())
}

/// containerd's service, which the release tarball does not ship.
fn write_unit() -> RuntimeResult<()> {
    let init = Init::detect();
    if !init.supervises() {
        return Ok(());
    }
    init.install_service(
        "containerd",
        &ServiceFile {
            systemd: CONTAINERD_UNIT.to_string(),
            openrc: CONTAINERD_OPENRC.to_string(),
        },
    )
    .map_err(|error| RuntimeError::Command(error.to_string()))?;
    Ok(())
}

fn start_containerd() -> RuntimeResult<()> {
    let init = Init::detect();
    if !init.supervises() {
        return Err(RuntimeError::Command(
            "nothing supervises services on this machine, so containerd cannot be \
             started for you — run `containerd` in the background and re-run install"
                .into(),
        ));
    }
    init.restart("containerd")
        .map_err(|error| RuntimeError::Command(error.to_string()))?;

    // Starting returns as soon as the process is spawned; the socket
    // is what the node actually needs, and it appears a moment later.
    for _ in 0..50 {
        if Path::new(SOCKET).exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err(RuntimeError::Command(format!(
        "containerd started but {SOCKET} never appeared — check its log"
    )))
}

fn run(program: &str, args: &[&str]) -> RuntimeResult<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| RuntimeError::Command(format!("could not run {program}: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(RuntimeError::Command(format!(
        "{program} {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

/// Where containerd's service file lives, whichever manager wants it.
pub fn containerd_service_path() -> std::path::PathBuf {
    Init::detect().service_path("containerd")
}

/// containerd's own unit, from its repository.
///
/// Reproduced rather than downloaded: it is short, it changes rarely,
/// and an install that fetched two files could fail between them and
/// leave binaries with nothing to supervise them.
///
/// The values that look arbitrary are upstream's and are load-bearing:
/// `LimitNOFILE=infinity` because containerd holds a descriptor per
/// container and per shim; `Delegate=yes` so containerd owns its
/// cgroup subtree rather than systemd reshuffling it underneath;
/// `KillMode=process` so stopping containerd does **not** kill the
/// shims, which is what lets a containerd upgrade leave running
/// containers alone.
pub const CONTAINERD_UNIT: &str = r#"# containerd's own unit, from github.com/containerd/containerd.
# Written by wabot-deploy because the release tarball ships binaries
# only. Left alone once written.
[Unit]
Description=containerd container runtime
Documentation=https://containerd.io
After=network.target dbus.service

[Service]
ExecStartPre=-/sbin/modprobe overlay
ExecStart=/usr/local/bin/containerd
Type=notify
Delegate=yes
KillMode=process
Restart=always
RestartSec=5
LimitNPROC=infinity
LimitCORE=infinity
TasksMax=infinity
OOMScoreAdjust=-999

[Install]
WantedBy=multi-user.target
"#;

/// The same service under OpenRC.
///
/// `supervise-daemon` for the same reason the unit says `Restart`:
/// containerd dying and staying dead takes every container with it at
/// the next boot.
///
/// What the unit expresses and this cannot: `KillMode=process`, which
/// is what lets a containerd restart leave the running shims alone.
/// OpenRC's supervisor stops the process it started and nothing else,
/// which happens to be the same behaviour — but by default rather than
/// by declaration, so it is written here where somebody will look.
pub const CONTAINERD_OPENRC: &str = r#"#!/sbin/openrc-run
# containerd, as wabot-deploy installs it. Written because the release
# tarball ships binaries only. Left alone once written.

name="containerd"
description="containerd container runtime"

command="/usr/local/bin/containerd"
supervisor="supervise-daemon"
pidfile="/run/containerd.pid"
respawn_delay=5
respawn_max=0

output_log="/var/log/containerd.log"
error_log="/var/log/containerd.log"

rc_ulimit="-n 1048576 -u unlimited"

depend() {
    need cgroups
    after net
}

start_pre() {
    # The snapshotter wants it, and Alpine does not load it on its own.
    modprobe overlay 2>/dev/null
    return 0
}
"#;

/// containerd's configuration, as this node needs it.
///
/// Deliberately short. Everything not set here is containerd's own
/// default, which is the right value and stays right when it changes.
pub const CONTAINERD_CONFIG: &str = r#"# Written by wabot-deploy. Edit if you must; it is written once, on
# the install that put containerd here, and never rewritten.
version = 3

# This section configures the *CRI* plugin — the Kubernetes-facing API.
# This node does not use it: it drives containerd's native API, where
# the runtime is chosen per container in `Containers.Create` rather
# than by configuration. Verified the hard way: with only this section
# set, `ctr run` still looked for runc and failed.
#
# It stays because it costs nothing and makes the tools someone will
# reach for on a broken node — crictl, nerdctl — use the same runtime
# this node does, rather than a runc that is not installed.
[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.runc.options]
  # The shim is named `runc.v2` but drives any runtime with a
  # runc-compatible CLI, and `BinaryName` picks which.
  BinaryName = "/usr/local/bin/crun"
  # Without this, memory limits and OOM accounting are wrong in ways
  # that only surface when a container is killed and the reason is
  # misreported.
  SystemdCgroup = true

[plugins.'io.containerd.cri.v1.images'.registry]
  config_path = "/etc/containerd/certs.d"
"#;

// A `registry_hosts_toml` helper lived here — the plain-HTTP host
// entry containerd needs to pull from the node's own registry over
// loopback. Removed until the registry exists to need it: it is
// documented in the architecture notes, and a config generator with no
// caller is a guess about a file nobody has written yet.

#[cfg(test)]
mod tests {
    use super::*;

    /// A pinned checksum that does not exist for an architecture we
    /// claim to support is an install that fails at the last step.
    #[test]
    fn every_supported_architecture_has_both_checksums() {
        for arch in ["x86_64", "aarch64"] {
            for what in ["containerd", "crun"] {
                let sum = checksum_for(what, arch)
                    .unwrap_or_else(|_| panic!("{what} on {arch} has no pinned checksum"));
                assert_eq!(sum.len(), 64, "{what}/{arch} is not a sha256");
                assert!(
                    sum.chars().all(|c| c.is_ascii_hexdigit()),
                    "{what}/{arch} is not hex"
                );
            }
        }
    }

    #[test]
    fn checksums_are_distinct() {
        let mut sums: Vec<&str> = CHECKSUMS.iter().map(|(_, _, sum)| *sum).collect();
        let total = sums.len();
        sums.sort_unstable();
        sums.dedup();
        assert_eq!(sums.len(), total, "two artifacts share a checksum");
    }

    #[test]
    fn release_names_are_the_ones_github_publishes() {
        assert_eq!(release_arch("x86_64"), "amd64");
        assert_eq!(release_arch("aarch64"), "arm64");
    }

    /// containerd's unit carries settings that look arbitrary and are
    /// not. `KillMode=process` is the one worth guarding: without it,
    /// stopping containerd kills every shim, and every container on
    /// the node dies with it.
    #[test]
    fn the_containerd_unit_keeps_its_load_bearing_settings() {
        for setting in [
            "KillMode=process",
            "Delegate=yes",
            "Type=notify",
            "ExecStart=/usr/local/bin/containerd",
            "Restart=always",
        ] {
            assert!(
                CONTAINERD_UNIT.contains(setting),
                "the containerd unit needs {setting}"
            );
        }
    }

    /// The two settings the node depends on, and would fail obscurely
    /// without: crun as the runtime, and systemd cgroups so an OOM is
    /// reported as one.
    #[test]
    fn the_configuration_selects_crun_and_systemd_cgroups() {
        assert!(CONTAINERD_CONFIG.contains(CRUN_PATH));
        assert!(CONTAINERD_CONFIG.contains("SystemdCgroup = true"));
        assert!(CONTAINERD_CONFIG.contains("version = 3"));
    }

    /// The configuration is CRI-only, and the comment has to keep
    /// saying so — a future reader who believes it selects crun for
    /// the native API will spend an afternoon on the same discovery.
    #[test]
    fn the_configuration_says_it_is_cri_only() {
        assert!(
            CONTAINERD_CONFIG.contains("does not use it"),
            "the CRI caveat must survive edits to this file"
        );
    }

    /// `status` runs on any machine, including one with none of it
    /// installed — a preflight that panics is worse than none.
    ///
    /// The check is an *implication*, not the formula restated: ready
    /// means every part is there. Written as an equality it was a copy
    /// of `ready()`'s body, and when `ready()` grew a fourth
    /// requirement — the CNI plugins — the copy kept the old
    /// definition and disagreed. It passed here anyway, because a
    /// machine with nothing installed makes both sides false; it took
    /// a CI runner, which ships containerd with Docker, to show it.
    #[test]
    fn status_is_never_ready_with_something_missing() {
        let status = status();

        if status.ready() {
            assert!(status.containerd.is_some(), "ready without containerd");
            assert!(status.crun.is_some(), "ready without crun");
            assert!(status.socket, "ready without a socket");
            assert!(status.cni, "ready without the CNI plugins");
        }
    }

    /// Every part is required, checked without depending on what the
    /// machine happens to have. This is the test that would have
    /// caught the CNI requirement being added to `ready()` while the
    /// check beside it kept the old definition.
    #[test]
    fn ready_means_every_part_is_there() {
        let complete = Status {
            containerd: Some("2.3.3".into()),
            crun: Some("1.28".into()),
            socket: true,
            cni: true,
        };
        assert!(complete.ready());

        let incomplete = [
            Status {
                containerd: None,
                ..complete.clone()
            },
            Status {
                crun: None,
                ..complete.clone()
            },
            Status {
                socket: false,
                ..complete.clone()
            },
            Status {
                cni: false,
                ..complete.clone()
            },
        ];
        for status in incomplete {
            assert!(!status.ready(), "ready with something missing: {status:?}");
        }
    }

    #[test]
    fn a_missing_program_has_no_version() {
        assert_eq!(version_of("wabot-no-such-program", &["--version"]), None);
    }
}
