//! Installing the binary and registering the service.
//!
//! ## `Type=notify`, not `simple`
//!
//! With `simple`, `systemctl start` returns as soon as the process is
//! *spawned* — so the install step that follows, or a dependent unit,
//! races the startup it was meant to wait for. The node calls
//! `sd_notify(READY=1)` once its listeners are bound, which is the
//! thing anyone waiting actually cares about.
//!
//! ## The unit is ours, the configuration is not
//!
//! `install` rewrites the unit on every run: it belongs to this
//! program and an operator editing it would lose the edit on the next
//! upgrade anyway. `config.toml` is the opposite — written once,
//! never touched again. The difference is which one an operator is
//! expected to own.

use std::path::{Path, PathBuf};

use super::init::{Init, ServiceFile};

pub const BINARY_PATH: &str = "/usr/local/bin/wabot-deploy";

/// What the service is called to whatever supervises it.
///
/// Without the `.service` suffix: systemd accepts the bare name and
/// OpenRC only knows the bare name, so one constant serves both.
pub const SERVICE_NAME: &str = "wabot-deploy";

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Command(String),
    #[error("could not find this executable: {0}")]
    NoSelf(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type ServiceResult<T> = Result<T, ServiceError>;

/// Copy this executable to its permanent home.
///
/// Returns whether anything changed. Compared by content, not by
/// timestamp: re-running `install` from the same binary should be a
/// no-op, and `cp` on every run would restart the service for nothing.
pub fn install_binary() -> ServiceResult<bool> {
    let current =
        std::env::current_exe().map_err(|error| ServiceError::NoSelf(error.to_string()))?;
    let target = Path::new(BINARY_PATH);

    if current == target {
        // Running the installed copy, installing over itself. Nothing
        // to do, and `copy` here would truncate the running binary.
        return Ok(false);
    }

    if target.exists() && same_contents(&current, target)? {
        return Ok(false);
    }

    std::fs::create_dir_all("/usr/local/bin")?;

    // Written beside the target and renamed, because `copy` onto a
    // running binary fails with ETXTBSY — and a rename is atomic, so
    // there is no moment where the path holds half a program.
    let staging = target.with_extension("new");
    std::fs::copy(&current, &staging)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&staging, target)?;
    Ok(true)
}

/// Are these the same binary?
///
/// Length first, because it settles almost every case for the price of
/// a stat — then the actual bytes, because an upgrade that happens to
/// preserve the size would otherwise be skipped and the operator would
/// be left running the old code with no sign of it.
fn same_contents(a: &Path, b: &Path) -> std::io::Result<bool> {
    if std::fs::metadata(a)?.len() != std::fs::metadata(b)?.len() {
        return Ok(false);
    }
    Ok(sha256_of(a)? == sha256_of(b)?)
}

fn sha256_of(path: &Path) -> std::io::Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finalize().into())
}

/// Write the service file and enable it. Returns whether it changed.
pub fn install_unit(config_path: &Path) -> ServiceResult<bool> {
    Init::detect().install_service(SERVICE_NAME, &service_file(config_path))
}

/// Start (or restart) the node.
pub fn start() -> ServiceResult<()> {
    Init::detect().restart(SERVICE_NAME)
}

/// Is the running service executing the binary that is installed?
///
/// `install_binary` renames a new file over the path, so a service
/// started before that keeps running the *old* inode — which is now
/// unlinked. The path resolves to the new file, the process holds the
/// old one, and nothing about either says so.
///
/// Compared by inode rather than by mtime or content: after the rename
/// there are two distinct files, and identity is exactly the question.
/// `false` whenever the answer cannot be established — the recovery is
/// a restart, and restarting a node that did not need it costs a
/// second.
pub fn running_current_binary() -> bool {
    let Some(pid) = main_pid() else {
        return false;
    };

    // Follows the symlink. A deleted target fails here, which is the
    // common case for "the binary was replaced under it".
    let Ok(running) = std::fs::metadata(format!("/proc/{pid}/exe")) else {
        return false;
    };
    let Ok(installed) = std::fs::metadata(BINARY_PATH) else {
        return false;
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        running.dev() == installed.dev() && running.ino() == installed.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (running, installed);
        false
    }
}

fn main_pid() -> Option<u32> {
    Init::detect().main_pid(SERVICE_NAME)
}

pub fn is_active() -> bool {
    Init::detect().is_active(SERVICE_NAME)
}

/// Is there anything here that can keep the node running?
pub fn supervised() -> bool {
    Init::detect().supervises()
}

/// The node's service, in both flavours, with the config path baked
/// in so `serve` and `install` cannot disagree about which file they
/// mean.
pub fn service_file(config_path: &Path) -> ServiceFile {
    ServiceFile {
        systemd: unit_file(config_path),
        openrc: openrc_file(config_path),
    }
}

/// The systemd unit.
pub fn unit_file(config_path: &Path) -> String {
    format!(
        r#"# Written by wabot-deploy. Rewritten on every install — this unit
# belongs to the program, unlike {config}, which is yours.
[Unit]
Description=wabot-deploy
Documentation=https://github.com/wabot-dev/wabot-deploy
After=network-online.target containerd.service
Wants=network-online.target
Requires=containerd.service

[Service]
Type=notify
ExecStart={binary} --config {config} serve
Restart=always
RestartSec=2

# The drain window the node is given on the way down. It defaults to
# three seconds outside production, which is short for a node holding
# open connections.
Environment=RUST_ENV=production
TimeoutStopSec=45

# The node binds 80 and 443, talks to containerd's socket, and writes
# its own data directory. It runs as root for now; narrowing this
# wants a dedicated user in containerd's group, and doing it wrong
# fails in ways that are hard to diagnose.
LimitNOFILE=65535
NoNewPrivileges=yes

# No PrivateTmp, ProtectHome or ProtectSystem, and the reason is not
# an oversight.
#
# Each of those puts the unit in its own mount namespace, and systemd
# makes that namespace a *slave* of the host's: mounts propagate in,
# never out. This node creates network namespaces — `/run/netns/<id>`,
# which is a bind mount — and containerd's shim, in a different unit,
# has to open them. From a slave namespace the shim sees only the empty
# placeholder file `ip netns` leaves behind, and refuses the container
# with `setns: Invalid argument`. `MountFlags=shared` does not fix it;
# the namespace stays slave to the host either way.
#
# So the trade is stated rather than discovered: a daemon whose job is
# building namespaces and mounts for the machine cannot be hidden from
# the machine's own mount tree.

[Install]
WantedBy=multi-user.target
"#,
        binary = BINARY_PATH,
        config = config_path.display(),
    )
}

/// The OpenRC init script — Alpine, and anything else that boots this
/// way.
///
/// `supervise-daemon` rather than the classic background-and-pidfile
/// mode, because that is what makes `Restart=always` true here too: a
/// node whose process died and stays dead is the failure this whole
/// service file exists to prevent. It also gives a pidfile holding the
/// *supervised* process, which is what `running_current_binary` needs.
///
/// `want containerd` and not `need`: a node whose containerd is
/// managed by something else must still boot. It comes up, and says
/// what is wrong on a page somebody can read — which beats a machine
/// that refuses to start the thing that would have told them.
pub fn openrc_file(config_path: &Path) -> String {
    format!(
        r#"#!/sbin/openrc-run
# Written by wabot-deploy. Rewritten on every install — this script
# belongs to the program, unlike {config}, which is yours.

name="wabot-deploy"
description="wabot-deploy"

command="{binary}"
command_args="--config {config} serve"

supervisor="supervise-daemon"
pidfile="/run/wabot-deploy.pid"
respawn_delay=2
# Unlimited: a node that gave up restarting is a node nobody is
# watching, which is the situation this is for.
respawn_max=0

# OpenRC has nowhere to send a daemon's output on its own, so it goes
# to a file rather than nowhere.
output_log="/var/log/wabot-deploy.log"
error_log="/var/log/wabot-deploy.log"

# The drain window the node is given on the way down. It defaults to
# three seconds outside production, which is short for a node holding
# open connections.
export RUST_ENV=production

depend() {{
    want containerd
    after net
}}
"#,
        binary = BINARY_PATH,
        config = config_path.display(),
    )
}

/// Where the service file would go, for reporting.
pub fn unit_path() -> PathBuf {
    Init::detect().service_path(SERVICE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "/etc/wabot-deploy/config.toml";

    fn unit() -> String {
        unit_file(Path::new(CONFIG))
    }

    /// `Type=notify` is the reason the install can start the node and
    /// know it is serving. With `simple` the next step would race it.
    #[test]
    fn the_unit_waits_for_readiness() {
        assert!(unit().contains("Type=notify"));
    }

    /// A node draining connections needs longer than the three
    /// seconds the framework gives a development process.
    #[test]
    fn the_unit_allows_a_real_drain() {
        let unit = unit();
        assert!(
            unit.contains("RUST_ENV=production"),
            "which is what widens the shutdown window"
        );
        assert!(unit.contains("TimeoutStopSec=45"));
    }

    /// The hardening that had to go, and must not come back.
    ///
    /// Each of these puts the unit in its own mount namespace, which
    /// systemd makes a slave of the host's: mounts propagate in, never
    /// out. The network namespaces this node builds then never reach
    /// containerd's shim, and every container is refused with `setns:
    /// Invalid argument` — an error that names neither systemd nor
    /// propagation, and cost two deploys to trace.
    #[test]
    fn the_unit_lets_network_namespaces_reach_containerd() {
        // Directives only: the comment in the unit names each of these
        // to explain their absence, and a plain `contains` would match
        // the explanation.
        let unit = unit();
        let directives: Vec<&str> = unit
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#'))
            .collect();

        for isolating in [
            "PrivateTmp=",
            "ProtectHome=",
            "ProtectSystem=",
            "PrivateMounts=",
        ] {
            assert!(
                !directives.iter().any(|line| line.starts_with(isolating)),
                "{isolating} puts the unit in a mount namespace that is slave to the \
                 host, so the network namespaces it creates never reach containerd's \
                 shim — which then refuses every container with `setns: Invalid argument`"
            );
        }
    }

    /// The node cannot start before containerd, and must come back
    /// after a crash.
    #[test]
    fn the_unit_orders_and_restarts() {
        let unit = unit();
        assert!(unit.contains("Requires=containerd.service"));
        assert!(unit.contains("After=network-online.target containerd.service"));
        assert!(unit.contains("Restart=always"));
        assert!(unit.contains("WantedBy=multi-user.target"));
    }

    /// The config path is baked in, so a node installed with
    /// `--config` elsewhere is served from there rather than from the
    /// default the unit would otherwise assume.
    #[test]
    fn the_config_path_reaches_the_unit() {
        let unit = unit_file(Path::new("/srv/node.toml"));
        assert!(unit.contains("--config /srv/node.toml serve"), "{unit}");
    }

    /// Rewriting an identical unit would reload systemd for nothing on
    /// every install.
    #[test]
    fn an_unchanged_unit_is_byte_identical() {
        assert_eq!(unit(), unit(), "the unit is deterministic");
    }

    /// The comparison an upgrade rests on. Same size and different
    /// bytes is the case a length check alone would get wrong, and it
    /// is exactly the case that matters: the operator would keep
    /// running the old binary with nothing to show why.
    #[test]
    fn identical_length_is_not_identical_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (a, b, c) = (
            dir.path().join("a"),
            dir.path().join("b"),
            dir.path().join("c"),
        );
        std::fs::write(&a, b"wabot-deploy v1").expect("write");
        std::fs::write(&b, b"wabot-deploy v2").expect("write");
        std::fs::write(&c, b"wabot-deploy v1").expect("write");

        assert!(
            !same_contents(&a, &b).expect("compare"),
            "same size, different bytes"
        );
        assert!(same_contents(&a, &c).expect("compare"));
    }

    /// What hardening is left after the mount-namespace ones had to
    /// go — see the test above for why they did.
    #[test]
    fn the_unit_keeps_the_hardening_that_costs_nothing() {
        let unit = unit();
        assert!(unit.contains("NoNewPrivileges=yes"));
        assert!(unit.contains("LimitNOFILE=65535"));
    }
}

#[cfg(test)]
mod openrc_tests {
    use super::*;
    use std::path::Path;

    const CONFIG: &str = "/etc/wabot-deploy/config.toml";

    fn script() -> String {
        openrc_file(Path::new(CONFIG))
    }

    /// OpenRC reads the shebang, and a script without it is a service
    /// that fails at boot with a message about a format.
    #[test]
    fn it_is_an_openrc_script() {
        assert!(script().starts_with("#!/sbin/openrc-run"));
    }

    /// The half of `Restart=always` that OpenRC does not give by
    /// default. A node whose process died and stays dead is the
    /// failure a service file exists to prevent.
    #[test]
    fn it_is_supervised_and_restarts_for_ever() {
        let script = script();
        assert!(script.contains("supervisor=\"supervise-daemon\""));
        assert!(script.contains("respawn_max=0"), "unlimited restarts");
    }

    /// `running_current_binary` reads this file to decide whether the
    /// node is executing the binary that is installed.
    #[test]
    fn it_names_the_pidfile_that_is_read_back() {
        assert!(script().contains("pidfile=\"/run/wabot-deploy.pid\""));
    }

    /// `want`, not `need`: a node whose containerd is managed by
    /// something else must still boot, and say what is wrong on a page
    /// somebody can read.
    #[test]
    fn containerd_is_wanted_not_required() {
        let script = script();
        assert!(script.contains("want containerd"));
        assert!(!script.contains("need containerd"));
    }

    /// The same widening the unit does — and for the same reason: the
    /// framework's default drain is three seconds outside production.
    #[test]
    fn it_widens_the_drain_window() {
        assert!(script().contains("RUST_ENV=production"));
    }

    /// Both flavours have to name the same config file and the same
    /// binary, or `install` and `serve` disagree about which node this
    /// is.
    #[test]
    fn both_flavours_agree_about_what_they_run() {
        let file = service_file(Path::new(CONFIG));
        for text in [&file.systemd, &file.openrc] {
            assert!(text.contains(BINARY_PATH), "{text}");
            assert!(text.contains(CONFIG), "{text}");
            assert!(text.contains("serve"), "{text}");
        }
    }
}
