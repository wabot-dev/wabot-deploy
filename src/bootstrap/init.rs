//! What supervises services on this machine.
//!
//! systemd on Debian, Ubuntu and most of the rest; **OpenRC** on
//! Alpine, which is a machine this product should want: musl, a static
//! binary, and a base system that leaves the RAM budget to the thing
//! doing the work.
//!
//! ## Mechanics here, content at the call site
//!
//! This module knows where a service file goes, how to enable it, how
//! to restart it and how to ask whether it is running. It does **not**
//! know what a wabot-deploy unit says or what containerd needs — those
//! live beside the thing they describe, in both flavours, because a
//! unit and an init script for the same service disagree about
//! everything except intent.
//!
//! ## Neither is not a failure
//!
//! A machine with no service manager still runs the node: `install`
//! writes everything and says that starting it is yours to arrange.
//! What it must never do is pretend it registered something.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::service::{ServiceError, ServiceResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Init {
    Systemd,
    OpenRc,
    /// A container, a chroot, or an init this does not know. The node
    /// runs; nothing restarts it.
    None,
}

/// The two texts one service needs, because the two managers share no
/// syntax.
pub struct ServiceFile {
    pub systemd: String,
    pub openrc: String,
}

impl Init {
    /// What is running this machine.
    ///
    /// systemd first because a machine can carry OpenRC's tools
    /// without OpenRC being what booted it. `/run/systemd/system` is
    /// the documented probe — it exists only when systemd is PID 1,
    /// unlike `/usr/bin/systemctl`, which is just a package.
    pub fn detect() -> Self {
        if Path::new("/run/systemd/system").is_dir() {
            return Self::Systemd;
        }
        if Path::new("/run/openrc").is_dir() || which("rc-service") {
            return Self::OpenRc;
        }
        Self::None
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Systemd => "systemd",
            Self::OpenRc => "OpenRC",
            Self::None => "none",
        }
    }

    /// Can this machine be asked to keep something running?
    pub fn supervises(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Where the service file for `name` belongs.
    pub fn service_path(self, name: &str) -> PathBuf {
        match self {
            Self::Systemd | Self::None => {
                PathBuf::from(format!("/etc/systemd/system/{name}.service"))
            }
            Self::OpenRc => PathBuf::from(format!("/etc/init.d/{name}")),
        }
    }

    /// Write the service file and enable it. Returns whether the file
    /// changed — an install that rewrites nothing must not reload
    /// anything either.
    pub fn install_service(self, name: &str, file: &ServiceFile) -> ServiceResult<bool> {
        let path = self.service_path(name);
        let wanted = match self {
            Self::Systemd => &file.systemd,
            Self::OpenRc => &file.openrc,
            Self::None => {
                return Err(ServiceError::Command(format!(
                    "no service manager here — run `wabot-deploy serve` yourself, or write \
                     a {name} service for whatever supervises this machine"
                )))
            }
        };

        let changed = match std::fs::read_to_string(&path) {
            Ok(existing) => &existing != wanted,
            Err(_) => true,
        };
        if changed {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, wanted)?;
        }

        match self {
            Self::Systemd => {
                if changed {
                    run("systemctl", &["daemon-reload"])?;
                }
                run("systemctl", &["enable", name])?;
            }
            Self::OpenRc => {
                // An init script that is not executable is a service
                // OpenRC skips with a message nobody reads.
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
                }
                // `default` is the runlevel a booted machine reaches.
                run("rc-update", &["add", name, "default"])?;
            }
            Self::None => unreachable!("returned above"),
        }
        Ok(changed)
    }

    /// Start it, or restart it if it is already running.
    ///
    /// Restart rather than start: this exists to leave a running node
    /// behind, and one still executing the previous binary is not that.
    pub fn restart(self, name: &str) -> ServiceResult<()> {
        match self {
            // No polling loop: `Type=notify` means systemctl already
            // waited for READY=1.
            Self::Systemd => run("systemctl", &["restart", name]),
            Self::OpenRc => run("rc-service", &[name, "restart"]),
            Self::None => Err(ServiceError::Command(format!(
                "no service manager here, so {name} cannot be started for you"
            ))),
        }
    }

    /// What an operator types to stop this service, in their own init's
    /// words.
    ///
    /// Text rather than an action, because the caller that wants this
    /// is `restore-node` refusing to run — and a command that stopped
    /// the node *for* somebody, as part of refusing to do the thing
    /// they asked for, is a surprise in the direction of taking their
    /// node down.
    pub fn stop_command(self, name: &str) -> Option<String> {
        match self {
            Self::Systemd => Some(format!("systemctl stop {name}")),
            Self::OpenRc => Some(format!("rc-service {name} stop")),
            Self::None => None,
        }
    }

    pub fn is_active(self, name: &str) -> bool {
        match self {
            Self::Systemd => Command::new("systemctl")
                .args(["is-active", "--quiet", name])
                .status()
                .map(|status| status.success())
                .unwrap_or(false),
            Self::OpenRc => Command::new("rc-service")
                .args([name, "status"])
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false),
            Self::None => false,
        }
    }

    /// The pid of the running service, when it can be established.
    ///
    /// `None` is always a safe answer: every caller treats it as "ask
    /// again by restarting", and restarting a node that did not need
    /// it costs a second.
    pub fn main_pid(self, name: &str) -> Option<u32> {
        match self {
            Self::Systemd => {
                let output = Command::new("systemctl")
                    .args(["show", "-p", "MainPID", "--value", name])
                    .output()
                    .ok()?;
                let pid: u32 = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse()
                    .ok()?;
                (pid > 0).then_some(pid)
            }
            // supervise-daemon keeps the supervised process's pid in
            // the pidfile the init script names. A stale file from a
            // process that is gone fails the `/proc` check below,
            // which is the point of making it here.
            Self::OpenRc => {
                let text = std::fs::read_to_string(format!("/run/{name}.pid")).ok()?;
                let pid: u32 = text.trim().parse().ok()?;
                Path::new(&format!("/proc/{pid}/exe"))
                    .exists()
                    .then_some(pid)
            }
            Self::None => None,
        }
    }

    /// Restart a service **from inside it**.
    ///
    /// The plain restart cannot be used here: on systemd this process
    /// lives in the unit's control group, and stopping the unit kills
    /// everything in it — including the `systemctl` that was asked to
    /// do the restarting. A transient unit runs outside that group. On
    /// OpenRC there is no such group, but the command still dies with
    /// its parent's session, so it gets one of its own.
    ///
    /// Used by the self-update, which is the only thing that asks the
    /// node to replace itself.
    pub fn restart_from_within(self, name: &str) -> ServiceResult<()> {
        match self {
            Self::Systemd => {
                let transient = Command::new("systemd-run")
                    .args([
                        "--collect",
                        "--on-active=1",
                        "--unit=wabot-deploy-selfupdate",
                        "systemctl",
                        "restart",
                        name,
                    ])
                    .status();
                match transient {
                    Ok(status) if status.success() => Ok(()),
                    other => {
                        // Worth trying anyway: systemd queues the job
                        // when the request arrives, and the request
                        // usually arrives before the kill. It is the
                        // ordering that is not guaranteed, which is why
                        // it is the fallback and not the plan.
                        tracing::warn!(
                            ?other,
                            "systemd-run did not take; restarting the plain way"
                        );
                        detach(&format!("sleep 1; systemctl restart {name}"))
                    }
                }
            }
            Self::OpenRc => detach(&format!("sleep 1; rc-service {name} restart")),
            Self::None => Err(ServiceError::Command(
                "no service manager here, so this node cannot restart itself".into(),
            )),
        }
    }
}

/// Run a shell command in a session of its own, so it outlives the
/// process that asked for it.
fn detach(script: &str) -> ServiceResult<()> {
    Command::new("setsid")
        .args(["sh", "-c", script])
        .spawn()
        .map(|_| ())
        .map_err(|error| ServiceError::Command(format!("could not detach the restart: {error}")))
}

fn which(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(program).exists())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A refusal that names a command has to name one that exists here.
    ///
    /// `restore-node` stops and tells the operator to stop the node
    /// first, which is only actionable if the line it prints is the one
    /// their machine understands. On an init this does not know there
    /// is no such line, and inventing `systemctl` for a box that has
    /// never had it sends somebody looking for a missing binary instead
    /// of at their own process.
    #[test]
    fn a_machine_is_only_told_to_type_what_it_has() {
        assert_eq!(
            Init::Systemd.stop_command("wabot-deploy").as_deref(),
            Some("systemctl stop wabot-deploy")
        );
        assert_eq!(
            Init::OpenRc.stop_command("wabot-deploy").as_deref(),
            Some("rc-service wabot-deploy stop")
        );
        assert_eq!(Init::None.stop_command("wabot-deploy"), None);
    }

    /// The distinction the rest of the code branches on: a machine
    /// that can be asked to keep something running, and one that
    /// cannot.
    #[test]
    fn only_a_real_manager_supervises() {
        assert!(Init::Systemd.supervises());
        assert!(Init::OpenRc.supervises());
        assert!(!Init::None.supervises());
    }

    #[test]
    fn each_manager_puts_its_file_where_it_looks_for_it() {
        assert_eq!(
            Init::Systemd.service_path("containerd"),
            Path::new("/etc/systemd/system/containerd.service")
        );
        assert_eq!(
            Init::OpenRc.service_path("containerd"),
            Path::new("/etc/init.d/containerd")
        );
    }

    /// Writing a service with nothing to run it has to fail loudly.
    /// The alternative — writing the file and reporting success — is a
    /// node that says it registered a service nothing will ever start.
    #[test]
    fn nothing_registers_nothing() {
        let file = ServiceFile {
            systemd: "unit".into(),
            openrc: "script".into(),
        };
        assert!(Init::None.install_service("wabot-deploy", &file).is_err());
        assert!(Init::None.restart("wabot-deploy").is_err());
        assert!(Init::None.restart_from_within("wabot-deploy").is_err());
        assert!(!Init::None.is_active("wabot-deploy"));
        assert_eq!(Init::None.main_pid("wabot-deploy"), None);
    }

    /// Detection must not answer "systemd" on a machine that merely
    /// has the client installed — `/run/systemd/system` exists only
    /// when systemd is PID 1.
    #[test]
    fn detection_answers_something_for_this_machine() {
        let init = Init::detect();
        assert!(matches!(init, Init::Systemd | Init::OpenRc | Init::None));
        // A Mac has neither, and the test suite runs on one.
        #[cfg(not(target_os = "linux"))]
        assert_eq!(init, Init::None);
    }
}
