//! From a row to a running container, and back.
//!
//! Everything above this module records *intent* — a service exists,
//! it should be running. Everything below it reports *fact* —
//! containerd has a task, or it does not. This is the only place the
//! two meet, and the only place allowed to disagree with either.
//!
//! ## Observed, not remembered
//!
//! The badge on a page comes from asking containerd, not from a column
//! somebody wrote when they pressed a button. A node that reports what
//! it was told is a node that lies after the first crash.
//!
//! ## Reconciling is the same code as deploying
//!
//! Boot does not have its own path: it asks, for each service that
//! should be running, whether it is — and deploys the ones that are
//! not. So the recovery after a reboot is the operation that was
//! tested by every deployment before it.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::platform::projects::Project;
use crate::platform::services::{self, DesiredState, Service};
use crate::platform::{projects, PlatformError};
use crate::runtime::client::Containerd;
use crate::runtime::containers::{self, TaskStatus};
use crate::runtime::network::{self, ProjectNetwork};
use crate::runtime::spec::ContainerRequest;

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("containerd: {0}")]
    Runtime(#[from] crate::runtime::client::ClientError),
    #[error("network: {0}")]
    Network(#[from] network::NetworkError),
    #[error(transparent)]
    Platform(#[from] PlatformError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub type DeployResult<T> = Result<T, DeployError>;

/// What a service is actually doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// A task exists and containerd says it is running.
    Running { pid: u32, address: Option<String> },
    /// A container exists, but its task is not running — it exited, or
    /// it never started.
    Stopped { exit_code: u32 },
    /// containerd has never heard of it.
    Absent,
    /// containerd could not be asked. Distinct from `Absent` on
    /// purpose: "the runtime is down" and "this service is not
    /// deployed" are different problems with different fixes, and
    /// collapsing them sends somebody to redeploy a healthy service.
    Unknown(String),
}

/// The node's deploy path.
///
/// Holds no containerd connection: it opens one per operation. A
/// deployment is seconds of work against a socket that may have been
/// restarted since the last one, and a cached channel that has to be
/// revalidated is more machinery than reconnecting.
pub struct Deployer {
    database: Arc<SqliteDatabase>,
    /// Written once at startup and bind-mounted into every container.
    resolv_conf: PathBuf,
}

impl Deployer {
    pub fn new(database: Arc<SqliteDatabase>, data_dir: &Path) -> Self {
        Self {
            database,
            resolv_conf: data_dir.join("resolv.conf"),
        }
    }

    /// Start (or restart) a service's container.
    ///
    /// Records the outcome on the row either way: an address and no
    /// error when it worked, the reason when it did not. A deployment
    /// that failed silently is one nobody can act on.
    pub async fn deploy(&self, project: &Project, service: &Service) -> DeployResult<Ipv4Addr> {
        let result = self.try_deploy(project, service).await;

        match &result {
            Ok(address) => {
                services::set_address(&self.database, &service.id, Some(&address.to_string()))
                    .await?;
                services::set_last_error(&self.database, &service.id, None).await?;
                services::set_desired_state(&self.database, &service.id, DesiredState::Running)
                    .await?;
            }
            Err(error) => {
                let message = error.to_string();
                tracing::error!(service = %service.slug, %message, "deploy failed");
                services::set_address(&self.database, &service.id, None).await?;
                services::set_last_error(&self.database, &service.id, Some(&message)).await?;
            }
        }
        result
    }

    async fn try_deploy(&self, project: &Project, service: &Service) -> DeployResult<Ipv4Addr> {
        let client = Containerd::connect().await?;
        let id = service.container_id(&project.slug);

        let index = projects::ensure_network_index(&self.database, &project.id).await?;
        let net = ProjectNetwork::new(index)?;

        // The container before the network: a create that fails after
        // the address is allocated would leak the reservation, and the
        // teardown below is what stops that from accumulating.
        containers::remove(&client, &id).await?;
        let address = network::attach(&net, &id).await?;

        self.write_resolv_conf()?;
        let request = ContainerRequest {
            command: Vec::new(),
            env: service
                .env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            port: service.container_port,
            network_ns: Some(network::netns_path(&id)),
            resolv_conf: Some(self.resolv_conf.clone()),
        };

        match containers::run(&client, &id, &service.image, &request).await {
            Ok(status) => {
                tracing::info!(
                    service = %service.slug,
                    project = %project.slug,
                    pid = status.pid,
                    %address,
                    "deployed"
                );
                Ok(address)
            }
            Err(error) => {
                // The namespace and the address go back, or the next
                // attempt starts from a half-built state and the /24
                // fills up with reservations for containers that never
                // existed.
                network::detach(&net, &id).await;
                Err(error.into())
            }
        }
    }

    /// Stop a service and take it off the network.
    ///
    /// Records `stopped` as the intent, so a reconcile does not start
    /// it again ten seconds later.
    pub async fn stop(&self, project: &Project, service: &Service) -> DeployResult<()> {
        services::set_desired_state(&self.database, &service.id, DesiredState::Stopped).await?;

        let id = service.container_id(&project.slug);
        let client = Containerd::connect().await?;

        containers::stop(&client, &id, STOP_GRACE).await?;
        containers::remove(&client, &id).await?;

        if let Some(index) = self.network_of(project).await {
            network::detach(&index, &id).await;
        }
        services::set_address(&self.database, &service.id, None).await?;
        services::set_last_error(&self.database, &service.id, None).await?;

        tracing::info!(service = %service.slug, project = %project.slug, "stopped");
        Ok(())
    }

    /// Remove everything a deleted service leaves behind.
    ///
    /// Best effort by design: the row is going either way, and a
    /// container we could not reach must not keep the operator from
    /// deleting the service they asked to delete. What it must not do
    /// is fail *silently* — hence the logs inside `stop`.
    pub async fn tear_down(&self, project: &Project, service: &Service) {
        if let Err(error) = self.stop(project, service).await {
            tracing::warn!(service = %service.slug, %error, "tearing down");
        }
    }

    /// What containerd says about this service right now.
    pub async fn observe(&self, project: &Project, service: &Service) -> Observed {
        let client = match Containerd::connect().await {
            Ok(client) => client,
            Err(error) => return Observed::Unknown(error.to_string()),
        };

        match containers::status(&client, &service.container_id(&project.slug)).await {
            Ok(Some(status)) if status.running() => Observed::Running {
                pid: status.pid,
                address: service.address.clone(),
            },
            Ok(Some(TaskStatus { exit_code, .. })) => Observed::Stopped { exit_code },
            Ok(None) => Observed::Absent,
            Err(error) => Observed::Unknown(error.to_string()),
        }
    }

    /// Bring the node's containers in line with its rows.
    ///
    /// Runs at startup. Only ever *starts* things: a container running
    /// that no row claims is left alone and reported, because deleting
    /// something the node does not understand is how a reconciler
    /// destroys data.
    pub async fn reconcile(&self) -> DeployResult<usize> {
        let services = services::all(&self.database, None).await?;
        let projects = projects::all(&self.database).await?;
        let mut started = 0;

        for service in services {
            if service.desired_state != DesiredState::Running {
                continue;
            }
            let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
                continue;
            };

            match self.observe(project, &service).await {
                Observed::Running { .. } => {}
                Observed::Unknown(error) => {
                    // Reconciling against a runtime that cannot answer
                    // would redeploy everything on the node because
                    // the socket was busy for a moment.
                    tracing::warn!(service = %service.slug, %error, "skipped: cannot ask containerd");
                }
                Observed::Absent | Observed::Stopped { .. } => {
                    tracing::info!(service = %service.slug, "reconciling: should be running");
                    if self.deploy(project, &service).await.is_ok() {
                        started += 1;
                    }
                }
            }
        }

        if started > 0 {
            tracing::info!(started, "reconciled");
        }
        Ok(started)
    }

    async fn network_of(&self, project: &Project) -> Option<ProjectNetwork> {
        let index = projects::ensure_network_index(&self.database, &project.id)
            .await
            .ok()?;
        ProjectNetwork::new(index).ok()
    }

    /// Write the resolver list containers get.
    ///
    /// The host's own `/etc/resolv.conf` usually cannot be reused: on
    /// anything running systemd-resolved it names `127.0.0.53`, which
    /// inside a container's own namespace is the container itself.
    /// systemd keeps the real upstreams in a second file, and that is
    /// the one to copy.
    fn write_resolv_conf(&self) -> std::io::Result<()> {
        let sources = [
            // systemd-resolved's upstream list, not its stub.
            "/run/systemd/resolve/resolv.conf",
            "/etc/resolv.conf",
        ];

        let nameservers: Vec<String> = sources
            .iter()
            .filter_map(|path| std::fs::read_to_string(path).ok())
            .find_map(|contents| {
                let servers = usable_nameservers(&contents);
                (!servers.is_empty()).then_some(servers)
            })
            .unwrap_or_else(|| {
                // Nothing usable on the host. A container with no
                // resolver fails in ways that look like the image's
                // fault, so the node picks two well-known ones and
                // says loudly that it did.
                tracing::warn!(
                    "no usable nameserver on this host; containers will use 1.1.1.1 and 8.8.8.8"
                );
                vec!["1.1.1.1".into(), "8.8.8.8".into()]
            });

        let body = format!(
            "# Written by wabot-deploy for containers. The host's own\n\
             # resolv.conf often names a loopback stub, which inside a\n\
             # container's namespace is the container.\n{}\n",
            nameservers
                .iter()
                .map(|server| format!("nameserver {server}"))
                .collect::<Vec<_>>()
                .join("\n")
        );

        if let Some(parent) = self.resolv_conf.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.resolv_conf, body)
    }
}

/// How long a container gets to exit before it is killed.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// The nameservers in a `resolv.conf` that a container could use.
///
/// Two kinds are dropped, both learned from a container that could not
/// resolve anything:
///
/// * **Loopback.** They name the host's own stub resolver, and inside
///   the container's namespace `127.0.0.53` is the container.
/// * **IPv6.** A project bridge is IPv4-only, so an IPv6 resolver is
///   unreachable from inside. Whether that matters depends on the
///   client: `wget` walks the list and falls through to the IPv4 ones,
///   `nslookup` tries the first and reports "Network unreachable". A
///   resolver list where DNS works for some programs and not others is
///   worse than a shorter one.
fn usable_nameservers(contents: &str) -> Vec<String> {
    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let address = line.strip_prefix("nameserver ")?.trim();
            let parsed: std::net::IpAddr = address.parse().ok()?;
            (parsed.is_ipv4() && !parsed.is_loopback()).then(|| address.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loopback_stub_is_not_a_nameserver_a_container_can_use() {
        let systemd_stub = "nameserver 127.0.0.53\noptions edns0 trust-ad\nsearch .\n";
        assert!(usable_nameservers(systemd_stub).is_empty());
    }

    #[test]
    fn real_nameservers_come_through_in_order() {
        let contents = "# comment\nnameserver 1.1.1.1\nnameserver 8.8.4.4\nsearch example.com\n";
        assert_eq!(usable_nameservers(contents), ["1.1.1.1", "8.8.4.4"]);
    }

    #[test]
    fn something_that_is_not_an_address_is_not_a_nameserver() {
        assert!(usable_nameservers("nameserver not-an-address").is_empty());
        assert!(usable_nameservers("").is_empty());
    }

    /// A container's bridge is IPv4-only, so an IPv6 resolver is a
    /// resolver that answers for `wget` and not for `nslookup`.
    #[test]
    fn ipv6_resolvers_are_dropped_while_the_bridge_is_ipv4() {
        let host = "nameserver 2a04:3540:53::1\nnameserver 94.237.127.9\n";
        assert_eq!(usable_nameservers(host), ["94.237.127.9"]);
        assert!(usable_nameservers("nameserver ::1").is_empty());
    }
}
