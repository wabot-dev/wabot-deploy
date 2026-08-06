//! The OCI runtime spec — the `config.json` crun reads.
//!
//! ## `Spec::default()` is not enough, and the gap is not obvious
//!
//! `oci-spec`'s default is a *valid* spec, not a *runnable* one. What
//! containerd's `oci.WithDefaultSpec` adds on top, and what a container
//! fails without:
//!
//! * `/proc`, `/dev`, `/dev/pts`, `/dev/shm`, `/dev/mqueue`, `/sys` —
//!   without `/proc` almost nothing starts; without `/dev/shm` a
//!   surprising number of runtimes fall over later, which is worse.
//! * the namespace set. Without it the container shares the host's PID
//!   and network namespaces and is not a container.
//! * masked and read-only paths. Without them `/proc/kcore` is
//!   readable from inside, which is host memory.
//! * the image's own `Env`, `Entrypoint`, `Cmd`, `WorkingDir`, `User`.
//!
//! Every failure here happens inside the shim, where the message is a
//! runtime error with no context. That is why this file is explicit
//! rather than clever.
//!
//! ## Network namespace, when there is one
//!
//! A deployed service gets its own, prepared by CNI before the
//! container is created, and the spec joins it by path. Then the port
//! inside the container is the port the image chose, and the proxy
//! reaches it at the container's address.
//!
//! Without one the container shares the host's network, which is what
//! `containerd --run` does for a throwaway check: no address to
//! allocate and no namespace to clean up, at the price of every
//! container seeing the host's ports.

use oci_spec::runtime::{
    Capability, LinuxBuilder, LinuxCapabilitiesBuilder, LinuxNamespace, LinuxNamespaceBuilder,
    LinuxNamespaceType, Mount, MountBuilder, ProcessBuilder, RootBuilder, Spec, SpecBuilder,
    UserBuilder,
};

use super::images::ImageConfig;

#[derive(Debug, thiserror::Error)]
pub enum SpecError {
    #[error("building the runtime spec: {0}")]
    Build(String),
    #[error("{0}")]
    Invalid(String),
}

type SpecResult<T> = Result<T, SpecError>;

/// What the node knows about a container beyond what its image says.
#[derive(Debug, Clone, Default)]
pub struct ContainerRequest {
    /// Overrides the image's command when set.
    pub command: Vec<String>,
    /// Added to the image's environment. Later wins on a repeated key,
    /// so these override the image's.
    pub env: Vec<(String, String)>,
    /// The port the application should listen on, handed to it as
    /// `PORT`. Most runtimes read it, and the ones that do not are
    /// configured with it anyway.
    pub port: Option<u16>,
    /// The network namespace to join, if any. `None` shares the
    /// host's.
    pub network_ns: Option<std::path::PathBuf>,
    /// A file to bind over `/etc/resolv.conf`.
    ///
    /// Needed exactly when `network_ns` is set: inside its own
    /// namespace the container cannot reach a resolver listening on
    /// the host's loopback, which is what `/etc/resolv.conf` names on
    /// any machine running systemd-resolved.
    pub resolv_conf: Option<std::path::PathBuf>,
}

/// Build the spec for one container.
pub fn build(image: &ImageConfig, request: &ContainerRequest) -> SpecResult<Spec> {
    let command = if request.command.is_empty() {
        image.command.clone()
    } else {
        request.command.clone()
    };
    if command.is_empty() {
        return Err(SpecError::Invalid(
            "neither the image nor the deployment says what to run".into(),
        ));
    }

    let process = ProcessBuilder::default()
        .args(command)
        .env(environment(image, request))
        .cwd(image.working_dir.clone().unwrap_or_else(|| "/".to_string()))
        .user(user(image)?)
        // The default set minus the ones a web application has no
        // business with. `NET_BIND_SERVICE` stays so a container can
        // listen below 1024 if it insists.
        .capabilities(
            LinuxCapabilitiesBuilder::default()
                .bounding(default_capabilities())
                .effective(default_capabilities())
                .permitted(default_capabilities())
                .build()
                .map_err(|error| SpecError::Build(error.to_string()))?,
        )
        .no_new_privileges(true)
        .build()
        .map_err(|error| SpecError::Build(error.to_string()))?;

    let linux = LinuxBuilder::default()
        .namespaces(namespaces(request.network_ns.as_deref())?)
        .masked_paths(masked_paths())
        .readonly_paths(readonly_paths())
        .build()
        .map_err(|error| SpecError::Build(error.to_string()))?;

    SpecBuilder::default()
        .version("1.0.2-dev")
        .process(process)
        // The rootfs is `rootfs`, relative to the bundle. containerd
        // passes the real mounts separately, in `CreateTaskRequest`.
        .root(
            RootBuilder::default()
                .path("rootfs")
                .readonly(false)
                .build()
                .map_err(|error| SpecError::Build(error.to_string()))?,
        )
        .hostname("wabot")
        .mounts(mounts(request.resolv_conf.as_deref()))
        .linux(linux)
        .build()
        .map_err(|error| SpecError::Build(error.to_string()))
}

/// The image's environment, then the node's, then `PORT`.
///
/// Order is the contract: later entries win, so a deployment can
/// override what the image baked in, and `PORT` wins over both because
/// the node is the one that knows which port it routed.
fn environment(image: &ImageConfig, request: &ContainerRequest) -> Vec<String> {
    let mut env = image.env.clone();

    // A `PATH` is not guaranteed by the image and its absence breaks
    // every command that is not an absolute path.
    if !env.iter().any(|entry| entry.starts_with("PATH=")) {
        env.push("PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into());
    }

    for (key, value) in &request.env {
        env.retain(|entry| !entry.starts_with(&format!("{key}=")));
        env.push(format!("{key}={value}"));
    }
    if let Some(port) = request.port {
        env.retain(|entry| !entry.starts_with("PORT="));
        env.push(format!("PORT={port}"));
    }
    env
}

/// The image's `User`, as far as it can be honoured without reading the
/// image's `/etc/passwd`.
///
/// A numeric id is used. A *name* is not: resolving it means mounting
/// the rootfs and parsing `passwd`, and guessing would run the process
/// as the wrong user. Root is the honest fallback, and it is what
/// happens today anyway for the common case of an image that says
/// nothing.
fn user(image: &ImageConfig) -> SpecResult<oci_spec::runtime::User> {
    let (uid, gid) = match image.user.as_deref() {
        None | Some("root") | Some("0") => (0, 0),
        Some(spec) => {
            let (uid, gid) = match spec.split_once(':') {
                Some((uid, gid)) => (uid, Some(gid)),
                None => (spec, None),
            };
            match uid.parse::<u32>() {
                Ok(uid) => {
                    let gid = gid.and_then(|gid| gid.parse().ok()).unwrap_or(uid);
                    (uid, gid)
                }
                // A username. Left as root with a warning rather than
                // guessed at: running as the wrong uid is worse than
                // running as the documented default.
                Err(_) => {
                    tracing::warn!(
                        user = spec,
                        "the image names a user this node cannot resolve; running as root"
                    );
                    (0, 0)
                }
            }
        }
    };

    UserBuilder::default()
        .uid(uid)
        .gid(gid)
        .build()
        .map_err(|error| SpecError::Build(error.to_string()))
}

/// The mounts every Linux container needs.
///
/// Not a preference — `/proc` alone is the difference between a
/// container that runs and one that exits before its first instruction.
fn mounts(resolv_conf: Option<&std::path::Path>) -> Vec<Mount> {
    let mount = |destination: &str, kind: &str, source: &str, options: &[&str]| {
        MountBuilder::default()
            .destination(destination)
            .typ(kind)
            .source(source)
            .options(options.iter().map(|o| o.to_string()).collect::<Vec<_>>())
            .build()
            .expect("a constant mount is well-formed")
    };

    let mut mounts = vec![
        mount("/proc", "proc", "proc", &["nosuid", "noexec", "nodev"]),
        mount(
            "/dev",
            "tmpfs",
            "tmpfs",
            &["nosuid", "strictatime", "mode=755", "size=65536k"],
        ),
        mount(
            "/dev/pts",
            "devpts",
            "devpts",
            &[
                "nosuid",
                "noexec",
                "newinstance",
                "ptmxmode=0666",
                "mode=0620",
                "gid=5",
            ],
        ),
        // 64 MB, which is Docker's default. Small enough to matter to
        // anything using shared memory seriously, and large enough
        // that nothing falls over on startup.
        mount(
            "/dev/shm",
            "tmpfs",
            "shm",
            &["nosuid", "noexec", "nodev", "mode=1777", "size=65536k"],
        ),
        mount(
            "/dev/mqueue",
            "mqueue",
            "mqueue",
            &["nosuid", "noexec", "nodev"],
        ),
        mount(
            "/sys",
            "sysfs",
            "sysfs",
            &["nosuid", "noexec", "nodev", "ro"],
        ),
    ];

    // Read-only: the file is the node's, shared by every container,
    // and one of them rewriting it would change every other's DNS.
    if let Some(path) = resolv_conf {
        mounts.push(mount(
            "/etc/resolv.conf",
            "bind",
            &path.to_string_lossy(),
            &["rbind", "ro", "nosuid", "noexec", "nodev"],
        ));
    }
    mounts
}

/// The namespaces that make this a container.
///
/// The network one is listed only when there is a path to join. A
/// `Network` namespace with no path means "make a fresh one", which
/// would be an isolated container with no route anywhere — the CNI
/// plugins have to have set it up beforehand for the address in it to
/// exist.
fn namespaces(network_ns: Option<&std::path::Path>) -> SpecResult<Vec<LinuxNamespace>> {
    let mut namespaces: Vec<LinuxNamespace> = [
        LinuxNamespaceType::Pid,
        LinuxNamespaceType::Ipc,
        LinuxNamespaceType::Uts,
        LinuxNamespaceType::Mount,
    ]
    .into_iter()
    .map(|typ| {
        LinuxNamespaceBuilder::default()
            .typ(typ)
            .build()
            .map_err(|error| SpecError::Build(error.to_string()))
    })
    .collect::<SpecResult<Vec<_>>>()?;

    if let Some(path) = network_ns {
        namespaces.push(
            LinuxNamespaceBuilder::default()
                .typ(LinuxNamespaceType::Network)
                .path(path)
                .build()
                .map_err(|error| SpecError::Build(error.to_string()))?,
        );
    }
    Ok(namespaces)
}

/// What a process inside gets to do.
///
/// The runc default set minus what a web application has no business
/// with — no `SYS_ADMIN`, no `SYS_MODULE`, no `MKNOD`.
fn default_capabilities() -> std::collections::HashSet<Capability> {
    [
        Capability::Chown,
        Capability::DacOverride,
        Capability::Fowner,
        Capability::Fsetid,
        Capability::Kill,
        Capability::Setgid,
        Capability::Setuid,
        Capability::Setpcap,
        // So a container can listen on 80 if it insists on it.
        Capability::NetBindService,
        Capability::SysChroot,
        Capability::AuditWrite,
    ]
    .into_iter()
    .collect()
}

/// Paths a container must not read.
///
/// `/proc/kcore` is host memory. The rest leak the host's kernel
/// configuration and hardware.
fn masked_paths() -> Vec<String> {
    [
        "/proc/acpi",
        "/proc/asound",
        "/proc/kcore",
        "/proc/keys",
        "/proc/latency_stats",
        "/proc/timer_list",
        "/proc/timer_stats",
        "/proc/sched_debug",
        "/proc/scsi",
        "/sys/firmware",
        "/sys/devices/virtual/powercap",
    ]
    .iter()
    .map(|path| path.to_string())
    .collect()
}

/// Paths a container may read and must not write.
fn readonly_paths() -> Vec<String> {
    [
        "/proc/bus",
        "/proc/fs",
        "/proc/irq",
        "/proc/sys",
        "/proc/sysrq-trigger",
    ]
    .iter()
    .map(|path| path.to_string())
    .collect()
}

/// The spec as the `Any` containerd stores on a container.
pub fn to_any(spec: &Spec) -> SpecResult<prost_types::Any> {
    let json = serde_json::to_vec(spec)
        .map_err(|error| SpecError::Build(format!("serializing the spec: {error}")))?;
    Ok(prost_types::Any {
        // The exact string containerd looks the spec up by.
        type_url: "types.containerd.io/opencontainers/runtime-spec/1/Spec".to_string(),
        value: json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nginx() -> ImageConfig {
        ImageConfig {
            command: vec!["/docker-entrypoint.sh".into(), "nginx".into()],
            env: vec!["NGINX_VERSION=1.27".into()],
            working_dir: None,
            user: None,
            exposed_ports: vec![80],
        }
    }

    /// Every mount here is one a container fails without, and `/proc`
    /// is the one it fails *immediately* without.
    #[test]
    fn the_spec_carries_the_mounts_a_container_needs() {
        let spec = build(&nginx(), &ContainerRequest::default()).expect("spec");
        let destinations: Vec<String> = spec
            .mounts()
            .as_ref()
            .expect("mounts")
            .iter()
            .map(|mount| mount.destination().display().to_string())
            .collect();

        for needed in [
            "/proc",
            "/dev",
            "/dev/pts",
            "/dev/shm",
            "/dev/mqueue",
            "/sys",
        ] {
            assert!(
                destinations.contains(&needed.to_string()),
                "missing {needed}"
            );
        }
    }

    /// Without these the container shares the host's PID space and is
    /// not isolated from anything.
    #[test]
    fn the_spec_isolates_pid_ipc_uts_and_mount() {
        let spec = build(&nginx(), &ContainerRequest::default()).expect("spec");
        let types: Vec<LinuxNamespaceType> = spec
            .linux()
            .as_ref()
            .expect("linux")
            .namespaces()
            .as_ref()
            .expect("namespaces")
            .iter()
            .map(|namespace| namespace.typ())
            .collect();

        for needed in [
            LinuxNamespaceType::Pid,
            LinuxNamespaceType::Ipc,
            LinuxNamespaceType::Uts,
            LinuxNamespaceType::Mount,
        ] {
            assert!(types.contains(&needed), "missing {needed:?}");
        }
    }

    /// The network namespace is deliberately *not* there: the container
    /// listens on a host port the edge proxies to. A test, because
    /// adding it later would silently break every route.
    #[test]
    fn the_network_namespace_is_shared_with_the_host() {
        let spec = build(&nginx(), &ContainerRequest::default()).expect("spec");
        let types: Vec<LinuxNamespaceType> = spec
            .linux()
            .as_ref()
            .expect("linux")
            .namespaces()
            .as_ref()
            .expect("namespaces")
            .iter()
            .map(|namespace| namespace.typ())
            .collect();
        assert!(
            !types.contains(&LinuxNamespaceType::Network),
            "a network namespace would make the container unreachable from the edge"
        );
    }

    /// `/proc/kcore` is host memory. Readable from inside a container
    /// is a whole-host compromise.
    #[test]
    fn host_memory_is_masked() {
        let spec = build(&nginx(), &ContainerRequest::default()).expect("spec");
        let masked = spec
            .linux()
            .as_ref()
            .expect("linux")
            .masked_paths()
            .clone()
            .expect("masked paths");
        assert!(masked.contains(&"/proc/kcore".to_string()));
        assert!(masked.contains(&"/sys/firmware".to_string()));
    }

    #[test]
    fn dangerous_capabilities_are_not_granted() {
        let spec = build(&nginx(), &ContainerRequest::default()).expect("spec");
        let effective = spec
            .process()
            .as_ref()
            .expect("process")
            .capabilities()
            .as_ref()
            .expect("capabilities")
            .effective()
            .clone()
            .expect("effective");

        for forbidden in [
            Capability::SysAdmin,
            Capability::SysModule,
            Capability::Mknod,
            Capability::SysPtrace,
        ] {
            assert!(
                !effective.contains(&forbidden),
                "a web application has no business with {forbidden:?}"
            );
        }
        assert!(
            effective.contains(&Capability::NetBindService),
            "a container that insists on port 80 should be able to"
        );
    }

    /// The precedence a deployment depends on: the image sets a
    /// default, the deployment overrides it, and PORT wins because the
    /// node is what chose the port.
    #[test]
    fn environment_precedence_is_image_then_request_then_port() {
        let mut image = nginx();
        image.env.push("PORT=9999".into());
        image.env.push("MODE=image".into());

        let spec = build(
            &image,
            &ContainerRequest {
                env: vec![("MODE".into(), "deployment".into())],
                port: Some(8080),
                ..Default::default()
            },
        )
        .expect("spec");

        let env = spec
            .process()
            .as_ref()
            .expect("process")
            .env()
            .clone()
            .expect("env");

        assert!(env.contains(&"MODE=deployment".to_string()));
        assert!(
            !env.contains(&"MODE=image".to_string()),
            "one MODE, not two"
        );
        assert!(env.contains(&"PORT=8080".to_string()));
        assert!(
            !env.contains(&"PORT=9999".to_string()),
            "the node's port wins over whatever the image baked in"
        );
        assert!(env.contains(&"NGINX_VERSION=1.27".to_string()));
    }

    /// Without a PATH, every command that is not an absolute path
    /// fails — inside the shim, with nothing useful said.
    #[test]
    fn a_path_is_always_present() {
        let spec = build(
            &ImageConfig {
                command: vec!["sh".into()],
                ..Default::default()
            },
            &ContainerRequest::default(),
        )
        .expect("spec");
        let env = spec
            .process()
            .as_ref()
            .expect("process")
            .env()
            .clone()
            .expect("env");
        assert!(env.iter().any(|entry| entry.starts_with("PATH=")));
    }

    #[test]
    fn the_deployment_can_override_the_images_command() {
        let spec = build(
            &nginx(),
            &ContainerRequest {
                command: vec!["/bin/sh".into(), "-c".into(), "sleep 1".into()],
                ..Default::default()
            },
        )
        .expect("spec");
        assert_eq!(
            spec.process().as_ref().unwrap().args().clone().unwrap(),
            vec!["/bin/sh", "-c", "sleep 1"]
        );
    }

    /// An image with no command and a deployment that does not supply
    /// one cannot run, and saying so here beats a runtime error from
    /// inside the shim.
    #[test]
    fn nothing_to_run_is_refused_before_containerd_sees_it() {
        let error = build(&ImageConfig::default(), &ContainerRequest::default())
            .expect_err("nothing to run");
        assert!(error.to_string().contains("what to run"), "{error}");
    }

    /// A numeric user is honoured; a name cannot be without the image's
    /// passwd file, and guessing would run the process as the wrong
    /// user.
    #[test]
    fn a_numeric_user_is_honoured_and_a_name_falls_back() {
        let numeric = build(
            &ImageConfig {
                command: vec!["x".into()],
                user: Some("1000:2000".into()),
                ..Default::default()
            },
            &ContainerRequest::default(),
        )
        .expect("spec");
        let user = numeric.process().as_ref().unwrap().user().clone();
        assert_eq!(user.uid(), 1000);
        assert_eq!(user.gid(), 2000);

        let named = build(
            &ImageConfig {
                command: vec!["x".into()],
                user: Some("nginx".into()),
                ..Default::default()
            },
            &ContainerRequest::default(),
        )
        .expect("spec");
        assert_eq!(named.process().as_ref().unwrap().user().clone().uid(), 0);
    }

    #[test]
    fn the_any_carries_the_type_url_containerd_looks_up() {
        let spec = build(&nginx(), &ContainerRequest::default()).expect("spec");
        let any = to_any(&spec).expect("any");
        assert_eq!(
            any.type_url,
            "types.containerd.io/opencontainers/runtime-spec/1/Spec"
        );
        // The value is the JSON crun reads, so it has to parse back.
        let round_tripped: serde_json::Value =
            serde_json::from_slice(&any.value).expect("valid JSON");
        assert!(round_tripped.get("process").is_some());
        assert_eq!(round_tripped["root"]["path"], "rootfs");
    }
}
