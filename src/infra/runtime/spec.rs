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
    Capability, LinuxBuilder, LinuxCapabilitiesBuilder, LinuxCpuBuilder, LinuxMemoryBuilder,
    LinuxNamespace, LinuxNamespaceBuilder, LinuxNamespaceType, LinuxResourcesBuilder, Mount,
    MountBuilder, ProcessBuilder, RootBuilder, Spec, SpecBuilder, UserBuilder,
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

/// A directory on the node, mounted into the container.
///
/// The node's half of `platform::volumes`: that module decides *which*
/// directory, this one puts it in the spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindMount {
    /// A path on the node. It has to exist — a bind of something that
    /// is not there fails inside the shim, where the message is about a
    /// mount rather than about the directory nobody created.
    pub source: std::path::PathBuf,
    /// Where it appears inside the container.
    pub destination: String,
    pub read_only: bool,
}

/// What the node knows about a container beyond what its image says.
#[derive(Debug, Clone, Default)]
pub struct ContainerRequest {
    /// Overrides the image's command when set.
    pub command: Vec<String>,
    /// Appended to whatever command runs, image's or overridden.
    ///
    /// The difference from `command` is the whole reason both exist: an
    /// image whose entrypoint ends in `exec postgres "$@"` is
    /// configured by adding `-c shared_buffers=32MB`, and replacing the
    /// command to do it would throw away the entrypoint that runs
    /// `initdb` on an empty data directory.
    pub args: Vec<String>,
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
    /// Directories from the node, mounted after everything else — so a
    /// destination inside one of the standard mounts would win, which
    /// is why `platform::volumes` refuses those destinations.
    pub mounts: Vec<BindMount>,
    /// The most memory this container may have, in bytes.
    ///
    /// `None` is what every container had until there were presets: no
    /// ceiling, and one process can take the machine.
    pub memory_limit: Option<u64>,
    /// The most CPU this container may have, in **millicores** — a
    /// thousand is one core, and 2500 is two and a half.
    ///
    /// Millicores rather than a fraction or a core count, because that
    /// is the unit `node::cpu` already reports in: a limit and a reading
    /// an operator compares should not need arithmetic between them. It
    /// is also the one absolute unit here — a percentage means something
    /// different on a one-core node and a thirty-two-core one, which is
    /// exactly the comparison somebody placing a service is making.
    ///
    /// `None` is no ceiling, which is what every container had. A
    /// container may then take every core, and on a one-core node that
    /// is the node — including the console somebody would use to stop
    /// it.
    pub cpu_millicores: Option<u32>,
    /// What this container was created with, for something later to
    /// compare against.
    ///
    /// containerd keeps these beside the container and hands them back
    /// on a read, so they live and die with it — which is what makes
    /// them the right home for "what was applied": a file would outlive
    /// the container it described, and a struct in this process would
    /// not survive a restart.
    pub labels: std::collections::BTreeMap<String, String>,
    /// The size of `/dev/shm`, in bytes. `None` is [`DEFAULT_SHM`].
    ///
    /// Its own field rather than a fraction of `memory_limit`, because
    /// the two answer to different things: the limit is what the
    /// operator chose, and this is what the engine inside needs. A
    /// database using parallel query is the case that cares, and 64 MB
    /// is the number it fails at.
    pub shm_size: Option<u64>,
}

/// Build the spec for one container.
pub fn build(image: &ImageConfig, request: &ContainerRequest) -> SpecResult<Spec> {
    let mut command = if request.command.is_empty() {
        image.command.clone()
    } else {
        request.command.clone()
    };
    if command.is_empty() {
        return Err(SpecError::Invalid(
            "neither the image nor the deployment says what to run".into(),
        ));
    }
    command.extend(request.args.iter().cloned());

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

    let mut linux = LinuxBuilder::default();
    linux = linux
        .namespaces(namespaces(request.network_ns.as_deref())?)
        .masked_paths(masked_paths())
        .readonly_paths(readonly_paths());
    if request.memory_limit.is_some() || request.cpu_millicores.is_some() {
        linux = linux.resources(resources(request.memory_limit, request.cpu_millicores)?);
    }
    let linux = linux
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
        .mounts(mounts(request))
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

/// Docker's `/dev/shm`, and so everybody's.
///
/// Small enough to matter to anything using shared memory seriously,
/// large enough that nothing falls over on startup — which is exactly
/// the size at which Postgres running a parallel query fails, and the
/// reason [`ContainerRequest::shm_size`] exists.
pub const DEFAULT_SHM: u64 = 64 * 1024 * 1024;

/// What the container may take.
///
/// `swap` is set to the same number as `limit`, which is how the OCI
/// spec says "no swap": the field is memory **plus** swap, so crun
/// writes `memory.swap.max = 0`. Left unset, a container over its
/// ceiling starts swapping instead of failing, and a database that is
/// quietly swapping is worse than one that was refused the memory —
/// the first is invisible until everything on the node is slow.
fn resources(
    memory_limit: Option<u64>,
    cpu_millicores: Option<u32>,
) -> SpecResult<oci_spec::runtime::LinuxResources> {
    let mut resources = LinuxResourcesBuilder::default();

    if let Some(limit) = memory_limit {
        let limit = i64::try_from(limit).map_err(|_| {
            SpecError::Invalid("that memory limit does not fit in the spec's own type".into())
        })?;
        resources = resources.memory(
            LinuxMemoryBuilder::default()
                .limit(limit)
                .swap(limit)
                .build()
                .map_err(|error| SpecError::Build(error.to_string()))?,
        );
    }

    if let Some(millicores) = cpu_millicores.filter(|millicores| *millicores > 0) {
        // `quota` microseconds of CPU in every `period` microseconds,
        // which crun writes as `cpu.max`. A hundred-millisecond period
        // is the kernel's own default and what everything else in this
        // world uses; changing it changes how *bursty* a container may
        // be, which is not what a ceiling is being asked for.
        //
        // Quota is allowed to exceed the period — that is how a limit
        // above one core is expressed, and 2500 millicores becomes
        // 250000/100000. Nothing here caps it at the machine's core
        // count: a ceiling larger than the machine is harmless and
        // refusing it would mean this file needing to know how many
        // cores the node has.
        let quota = i64::from(millicores) * (CPU_PERIOD as i64) / 1_000;
        resources = resources.cpu(
            LinuxCpuBuilder::default()
                .period(CPU_PERIOD)
                .quota(quota)
                .build()
                .map_err(|error| SpecError::Build(error.to_string()))?,
        );
    }

    resources
        .build()
        .map_err(|error| SpecError::Build(error.to_string()))
}

/// The window a CPU quota is measured over, in microseconds.
///
/// The kernel's default. A shorter one makes a container that bursts
/// stutter; a longer one lets it take the machine for a noticeable
/// moment before it is throttled.
const CPU_PERIOD: u64 = 100_000;

/// The mounts every Linux container needs, then the node's own.
///
/// Not a preference — `/proc` alone is the difference between a
/// container that runs and one that exits before its first instruction.
fn mounts(request: &ContainerRequest) -> Vec<Mount> {
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
        // Sized by the caller. A tmpfs page is charged to the cgroup
        // that wrote it, so this is a cap and not a reservation — a
        // container with a memory limit cannot escape it through here.
        mount(
            "/dev/shm",
            "tmpfs",
            "shm",
            &[
                "nosuid",
                "noexec",
                "nodev",
                "mode=1777",
                &format!("size={}k", request.shm_size.unwrap_or(DEFAULT_SHM) / 1024),
            ],
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
    if let Some(path) = request.resolv_conf.as_deref() {
        mounts.push(mount(
            "/etc/resolv.conf",
            "bind",
            &path.to_string_lossy(),
            &["rbind", "ro", "nosuid", "noexec", "nodev"],
        ));
    }

    // Last, so a volume lands on top of the rootfs rather than under
    // one of the mounts above. `platform::volumes` refuses those
    // destinations for that reason; this is the half that makes the
    // refusal necessary.
    //
    // `nosuid` and `nodev` on every one: storage a container writes has
    // no business carrying a setuid binary or a device node, and
    // neither does storage the node hands it.
    for bind in &request.mounts {
        let mut options = vec!["rbind", "nosuid", "nodev"];
        options.push(match bind.read_only {
            true => "ro",
            false => "rw",
        });
        mounts.push(mount(
            &bind.destination,
            "bind",
            &bind.source.to_string_lossy(),
            &options,
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

    /// A volume is what makes a database possible: the snapshot is
    /// removed on every deployment, and this is the part that is not.
    #[test]
    fn a_volume_is_bound_in_after_the_mounts_a_container_needs() {
        let spec = build(
            &nginx(),
            &ContainerRequest {
                mounts: vec![BindMount {
                    source: "/var/lib/wabot-deploy/volumes/demo.db/data".into(),
                    destination: "/var/lib/postgresql/data".into(),
                    read_only: false,
                }],
                ..Default::default()
            },
        )
        .expect("spec");

        let mounts = spec.mounts().clone().expect("mounts");
        let volume = mounts
            .iter()
            .find(|mount| mount.destination().ends_with("postgresql/data"))
            .expect("the volume is mounted");

        assert_eq!(volume.typ().as_deref(), Some("bind"));
        assert_eq!(
            volume.source().as_deref().map(|p| p.display().to_string()),
            Some("/var/lib/wabot-deploy/volumes/demo.db/data".to_string())
        );
        let options = volume.options().clone().expect("options");
        assert!(options.contains(&"rbind".to_string()));
        assert!(options.contains(&"rw".to_string()));
        assert!(
            options.contains(&"nosuid".to_string()) && options.contains(&"nodev".to_string()),
            "storage has no business carrying a setuid binary or a device node"
        );

        // After `/proc` and the rest: mounts are applied in order, so a
        // volume listed first could be shadowed by one of them.
        let position = |destination: &str| {
            mounts
                .iter()
                .position(|mount| mount.destination().display().to_string() == destination)
        };
        assert!(position("/var/lib/postgresql/data") > position("/proc"));
    }

    #[test]
    fn a_read_only_mount_says_so() {
        let spec = build(
            &nginx(),
            &ContainerRequest {
                mounts: vec![BindMount {
                    source: "/var/lib/wabot-deploy/config/demo.db".into(),
                    destination: "/etc/wabot".into(),
                    read_only: true,
                }],
                ..Default::default()
            },
        )
        .expect("spec");

        let options = spec
            .mounts()
            .clone()
            .expect("mounts")
            .into_iter()
            .find(|mount| mount.destination().display().to_string() == "/etc/wabot")
            .expect("mounted")
            .options()
            .clone()
            .expect("options");
        assert!(options.contains(&"ro".to_string()));
        assert!(!options.contains(&"rw".to_string()), "both would be a lie");
    }

    /// A CPU ceiling reaches the spec as a quota over a period, which
    /// is what crun writes into `cpu.max`.
    ///
    /// The half that shipped without the other: `memory.max` was
    /// written and nothing wrote this, so a container that could not
    /// take the machine's memory could still take every core it had —
    /// and on a one-core node that is the console, the edge and the
    /// deploy path with it.
    ///
    /// Millicores in, microseconds out. A quota **larger** than the
    /// period is how a ceiling above one core is expressed, and nothing
    /// caps it at the machine's cores: a limit bigger than the machine
    /// is harmless, and refusing it would mean this file knowing how
    /// many cores the node has.
    #[test]
    fn a_cpu_limit_reaches_the_spec_as_a_quota_over_a_period() {
        let quota_for = |millicores| {
            let spec = build(
                &nginx(),
                &ContainerRequest {
                    cpu_millicores: Some(millicores),
                    ..Default::default()
                },
            )
            .expect("spec");
            let cpu = spec
                .linux()
                .as_ref()
                .expect("linux")
                .resources()
                .as_ref()
                .expect("resources")
                .cpu()
                .clone()
                .expect("cpu");
            (cpu.quota(), cpu.period())
        };

        // Half a core: half the period.
        assert_eq!(quota_for(500), (Some(50_000), Some(100_000)));
        assert_eq!(quota_for(1_000), (Some(100_000), Some(100_000)));
        // And two and a half, which is a quota over the period.
        assert_eq!(quota_for(2_500), (Some(250_000), Some(100_000)));
    }

    /// The two ceilings are independent, and either alone still writes
    /// `resources`.
    ///
    /// Guarded because the branch that decides is one `if` over two
    /// fields: an earlier version wrote resources only when memory was
    /// set, so a service with a CPU ceiling and no memory one got
    /// neither — silently, which is the worst way to not have a limit.
    #[test]
    fn either_ceiling_alone_still_reaches_the_spec() {
        let resources = |request| {
            build(&nginx(), &request)
                .expect("spec")
                .linux()
                .as_ref()
                .expect("linux")
                .resources()
                .clone()
        };

        let cpu_only = resources(ContainerRequest {
            cpu_millicores: Some(500),
            ..Default::default()
        })
        .expect("resources");
        assert!(cpu_only.cpu().is_some());
        assert!(
            cpu_only.memory().is_none(),
            "and nothing it did not ask for"
        );

        let memory_only = resources(ContainerRequest {
            memory_limit: Some(64 * 1024 * 1024),
            ..Default::default()
        })
        .expect("resources");
        assert!(memory_only.memory().is_some());
        assert!(memory_only.cpu().is_none());

        // A container that asked for neither is unbounded, as it always
        // was. Not `resources().is_none()`: the builder fills that in
        // with the device rules every container gets, so the claim is
        // about the two fields this writes and nothing else.
        let neither = resources(ContainerRequest::default()).expect("the device rules");
        assert!(neither.memory().is_none());
        assert!(neither.cpu().is_none());
    }

    /// The ceiling the presets exist to set. `swap` carries the same
    /// number because the field is memory *plus* swap — so equal means
    /// none, and a container over its limit fails rather than quietly
    /// swapping the node into the ground.
    #[test]
    fn a_memory_limit_reaches_the_spec_with_swap_turned_off() {
        let spec = build(
            &nginx(),
            &ContainerRequest {
                memory_limit: Some(128 * 1024 * 1024),
                ..Default::default()
            },
        )
        .expect("spec");

        let memory = spec
            .linux()
            .as_ref()
            .expect("linux")
            .resources()
            .as_ref()
            .expect("resources")
            .memory()
            .expect("memory");

        assert_eq!(memory.limit(), Some(134_217_728));
        assert_eq!(
            memory.swap(),
            Some(134_217_728),
            "memory + swap equal to memory is what no swap means"
        );
    }

    /// No preset is still the old behaviour, deliberately: every
    /// service that exists today runs without a ceiling, and a default
    /// would take a node's containers down to introduce a setting.
    ///
    /// The assertion is on the *memory*, not on `resources` being
    /// absent: `LinuxBuilder` fills that in with an empty block of its
    /// own, and an empty block is what "no limits" looks like in the
    /// spec.
    #[test]
    fn a_container_with_no_preset_has_no_ceiling() {
        let spec = build(&nginx(), &ContainerRequest::default()).expect("spec");
        let memory = spec
            .linux()
            .as_ref()
            .expect("linux")
            .resources()
            .as_ref()
            .and_then(|resources| resources.memory().as_ref().and_then(|m| m.limit()));
        assert!(
            memory.is_none(),
            "a service that never asked for a ceiling was given one"
        );
    }

    /// 64 MB of `/dev/shm` is where Postgres running a parallel query
    /// fails, and it was hard-coded.
    #[test]
    fn dev_shm_follows_the_preset_and_defaults_to_dockers_size() {
        let size_of = |request| {
            build(&nginx(), &request)
                .expect("spec")
                .mounts()
                .clone()
                .expect("mounts")
                .into_iter()
                .find(|mount| mount.destination().display().to_string() == "/dev/shm")
                .expect("shm")
                .options()
                .clone()
                .expect("options")
                .into_iter()
                .find_map(|option| {
                    option
                        .strip_prefix("size=")
                        .map(|size| size.trim_end_matches('k').to_string())
                })
                .expect("a size")
        };

        assert_eq!(size_of(ContainerRequest::default()), "65536");
        assert_eq!(
            size_of(ContainerRequest {
                shm_size: Some(256 * 1024 * 1024),
                ..Default::default()
            }),
            "262144"
        );
    }

    /// The difference between the two command fields, and the reason
    /// both exist: `postgres -c shared_buffers=32MB` has to keep the
    /// entrypoint that runs `initdb` on an empty data directory.
    #[test]
    fn arguments_are_appended_to_the_images_own_command() {
        let spec = build(
            &ImageConfig {
                command: vec!["docker-entrypoint.sh".into(), "postgres".into()],
                ..Default::default()
            },
            &ContainerRequest {
                args: vec!["-c".into(), "shared_buffers=32MB".into()],
                ..Default::default()
            },
        )
        .expect("spec");

        assert_eq!(
            spec.process().as_ref().unwrap().args().clone().unwrap(),
            vec![
                "docker-entrypoint.sh",
                "postgres",
                "-c",
                "shared_buffers=32MB"
            ]
        );
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
