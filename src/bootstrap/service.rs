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
use std::process::Command;

pub const BINARY_PATH: &str = "/usr/local/bin/wabot-deploy";
pub const UNIT_PATH: &str = "/etc/systemd/system/wabot-deploy.service";
pub const UNIT_NAME: &str = "wabot-deploy.service";

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    Command(String),
    #[error("could not find this executable: {0}")]
    NoSelf(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

type ServiceResult<T> = Result<T, ServiceError>;

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

/// Write the unit and enable it. Returns whether the unit changed.
pub fn install_unit(config_path: &Path) -> ServiceResult<bool> {
    if !systemd_available() {
        return Err(ServiceError::Command(
            "no systemd here — run `wabot-deploy serve` yourself, or install a unit for \
             whatever supervises services on this machine"
                .into(),
        ));
    }

    let unit = unit_file(config_path);
    let changed = match std::fs::read_to_string(UNIT_PATH) {
        Ok(existing) => existing != unit,
        Err(_) => true,
    };

    if changed {
        std::fs::write(UNIT_PATH, &unit)?;
        run("systemctl", &["daemon-reload"])?;
    }
    run("systemctl", &["enable", UNIT_NAME])?;
    Ok(changed)
}

/// Start (or restart) the node and wait for it to be ready.
///
/// `restart` rather than `start`: the step exists to leave a running
/// node behind, and one already running an older binary is not that.
pub fn start() -> ServiceResult<()> {
    run("systemctl", &["restart", UNIT_NAME])?;
    // No polling loop: `Type=notify` means systemctl already waited
    // for READY=1. That is the whole reason for the unit type.
    Ok(())
}

pub fn is_active() -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", UNIT_NAME])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn systemd_available() -> bool {
    Path::new("/run/systemd/system").is_dir()
}

/// The unit, with the config path baked in so `serve` and `install`
/// cannot disagree about which file they mean.
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
ProtectHome=yes
ProtectSystem=full
PrivateTmp=yes

[Install]
WantedBy=multi-user.target
"#,
        binary = BINARY_PATH,
        config = config_path.display(),
    )
}

fn run(program: &str, args: &[&str]) -> ServiceResult<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| ServiceError::Command(format!("could not run {program}: {error}")))?;
    if output.status.success() {
        return Ok(());
    }
    Err(ServiceError::Command(format!(
        "{program} {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

/// Where the unit would go, for reporting.
pub fn unit_path() -> PathBuf {
    PathBuf::from(UNIT_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit() -> String {
        unit_file(Path::new("/etc/wabot-deploy/config.toml"))
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

    #[test]
    fn the_unit_hardening_is_present_but_not_absurd() {
        let unit = unit();
        assert!(unit.contains("NoNewPrivileges=yes"));
        assert!(unit.contains("ProtectHome=yes"));
        // Not `ProtectSystem=strict`: the node writes /etc and
        // /usr/local/bin during an in-place upgrade.
        assert!(unit.contains("ProtectSystem=full"));
    }
}
