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

pub mod dns;
pub mod jobs;
pub mod routing;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::platform::ports::{self, Port};
use crate::platform::projects::Project;
use crate::platform::releases::{self, Release};
use crate::platform::replicas::{self, Replica};
use crate::platform::services::{self, DesiredState, Service};
use crate::platform::{projects, PlatformError};
use crate::runtime::client::Containerd;
use crate::runtime::containers::{self, TaskStatus};
use crate::runtime::network::{self, PortMapping, ProjectNetwork};
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
    /// Something the caller asked for that this node will not do — a
    /// service with nothing placed here, and nothing to deploy.
    #[error("{0}")]
    Refused(String),
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
    /// The whole config, because the node's domain can change while it
    /// runs and is read per use rather than captured here.
    config: crate::config::Config,
    /// The live table the listener reads, when there is one. `None` in
    /// a test and in `install`, where nothing is serving.
    routes: Option<Arc<crate::edge::routes::RouteTable>>,
    /// How to tell the certificate loop that a hostname appeared.
    certificates: Option<Arc<crate::edge::acme::Wake>>,
}

impl Deployer {
    pub fn new(database: Arc<SqliteDatabase>, config: &crate::config::Config) -> Self {
        Self {
            database,
            resolv_conf: config.node.data_dir.join("resolv.conf"),
            config: config.clone(),
            routes: None,
            certificates: None,
        }
    }

    /// Hand the deployer the table the edge is serving from, so a
    /// deployment takes effect without a restart.
    pub fn with_routes(mut self, routes: Arc<crate::edge::routes::RouteTable>) -> Self {
        self.routes = Some(routes);
        self
    }

    /// Hand the deployer the certificate loop's doorbell, so a service
    /// given a hostname does not wait out the renewal interval for a
    /// certificate nobody asked for on its behalf.
    pub fn with_certificates(mut self, wake: Arc<crate::edge::acme::Wake>) -> Self {
        self.certificates = Some(wake);
        self
    }

    /// Recompute the routes. Called after anything that can change
    /// where a hostname points.
    pub(crate) async fn sync_routes(&self) {
        if let Err(error) = routing::sync(
            &self.database,
            crate::node::settings::domain(&self.database, &self.config)
                .await
                .as_deref(),
            self.routes.as_ref(),
        )
        .await
        {
            tracing::error!(%error, "could not update the routes");
        }

        // The routes and the certificates answer for the same names, so
        // whatever changed one has to let the other look again.
        if let Some(wake) = &self.certificates {
            wake.now();
        }
    }

    /// Start (or restart) every copy of a service that runs here.
    ///
    /// The service is what to run; a replica is one running copy of it.
    /// Returns the address of the first one, which is what a caller
    /// that used to get "the" address wants — and what routing used to
    /// read before it learned to gather them.
    pub async fn deploy(&self, project: &Project, service: &Service) -> DeployResult<Ipv4Addr> {
        let mine = self.mine(service).await?;
        let Some(first) = mine.first().cloned() else {
            return Err(DeployError::Refused(format!(
                "{} has no replica on this node",
                service.slug
            )));
        };

        // The intent is the service's, not one copy's: `stopped` has to
        // mean the whole service, or a reconcile would start back the
        // replicas somebody did not stop.
        services::set_desired_state(&self.database, &service.id, DesiredState::Running).await?;

        let mut first_address = None;
        let mut failure = None;
        for replica in &mine {
            match self.deploy_one(project, service, replica).await {
                Ok(address) if replica.id == first.id => first_address = Some(address),
                Ok(_) => {}
                Err(error) => failure = Some(error),
            }
        }

        // The routes go up once, after all of them: rebuilding per
        // replica would publish a half-deployed service n times.
        self.sync_routes().await;

        match (first_address, failure) {
            (Some(address), _) => Ok(address),
            (None, Some(error)) => Err(error),
            (None, None) => Err(DeployError::Refused(format!(
                "{} did not start anywhere",
                service.slug
            ))),
        }
    }

    /// One copy, with its outcome on its own row.
    ///
    /// Per replica and not per service: one copy failing to pull is not
    /// the service failing, and a page that said so would send somebody
    /// looking at the wrong container.
    async fn deploy_one(
        &self,
        project: &Project,
        service: &Service,
        replica: &Replica,
    ) -> DeployResult<Ipv4Addr> {
        let result = self.try_deploy(project, service, replica).await;

        match &result {
            Ok(address) => {
                replicas::set_address(&self.database, &replica.id, Some(&address.to_string()))
                    .await?;
                replicas::set_last_error(&self.database, &replica.id, None).await?;
            }
            Err(error) => {
                let message = error.to_string();
                tracing::error!(service = %service.slug, slot = replica.slot, %message, "deploy failed");
                replicas::set_address(&self.database, &replica.id, None).await?;
                replicas::set_last_error(&self.database, &replica.id, Some(&message)).await?;
            }
        }
        result
    }

    /// The copies of a service this node is the one running.
    async fn mine(&self, service: &Service) -> DeployResult<Vec<Replica>> {
        Ok(replicas::of_service(&self.database, &service.id)
            .await?
            .into_iter()
            .filter(|replica| replica.is_here() && !replica.evicted())
            .collect())
    }

    async fn try_deploy(
        &self,
        project: &Project,
        service: &Service,
        replica: &Replica,
    ) -> DeployResult<Ipv4Addr> {
        let client = Containerd::connect().await?;
        let id = replica.container_id(&project.slug, &service.slug);

        let index = projects::ensure_network_index(&self.database, &project.id).await?;
        let net = ProjectNetwork::new(index)?;

        // The container before the network: a create that fails after
        // the address is allocated would leak the reservation, and the
        // teardown below is what stops that from accumulating.
        containers::remove(&client, &id).await?;

        let ports = ports::of_service(&self.database, &service.id).await?;
        let published: Vec<PortMapping> = ports
            .iter()
            .filter_map(|port| {
                Some(PortMapping {
                    host_port: port.host_port?,
                    container_port: port.container_port,
                    // Every interface: this is the operator asking for
                    // the port to be reachable, which is what
                    // publishing means.
                    host_ip: None,
                })
            })
            .collect();

        // A port for an edge somewhere else, when this service has a
        // name to serve. Bound to this node's overlay address and
        // nowhere else: the container stays on the private network, and
        // what can reach it is a node on the overlay — not the
        // internet, which a `0.0.0.0` port would have handed it to.
        //
        // A port with a hostname *here*, or any port at all on a
        // service that belongs to another node. The second half is not
        // a special case: a service that arrived on an errand has no
        // hostname here — the name belongs to the node that placed it,
        // and claiming it here would be this node answering for
        // somebody else's name. Reading only the first condition left
        // every replica placed on another node with no way in at all,
        // which was the whole point of placing it.
        //
        // A service of this node's own with no name is still skipped:
        // nothing would proxy to it, and opening a port for it would be
        // opening one nobody asked for.
        let mut published = published;
        let reachable = ports
            .iter()
            .find(|port| port.hostname.is_some())
            .or_else(|| match service.is_ours() {
                true => None,
                false => ports.first(),
            });
        if let Some(port) = reachable {
            // Not on an overlay means nothing elsewhere can reach this
            // node anyway, so there is nothing to open a port for.
            if let Some(overlay) = self.overlay_address().await {
                let overlay_port =
                    replicas::ensure_overlay_port(&self.database, &replica.id).await?;
                published.push(PortMapping {
                    host_port: overlay_port,
                    container_port: port.container_port,
                    host_ip: Some(overlay),
                });
            }
        }

        let address = network::attach(&net, &id, &published).await?;

        self.write_resolv_conf()?;
        let request = ContainerRequest {
            command: Vec::new(),
            env: service
                .env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            port: primary_port(&ports),
            network_ns: Some(network::netns_path(&id)),
            resolv_conf: Some(self.resolv_conf.clone()),
        };

        // What this node presents to the registry the image lives on,
        // if it has been given anything. `None` is the ordinary case —
        // an image from a registry that serves anybody.
        let credential =
            crate::platform::registry_credentials::for_reference(&self.database, &service.image)
                .await;

        match containers::run(&client, &id, &service.image, &request, credential.as_ref()).await {
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

    /// Bring a service back to what it was running.
    ///
    /// Its release when it has one, and its image reference only when
    /// it does not. The difference matters the moment a tag moves: a
    /// restart that resolved the tag again would quietly bring in
    /// whatever was pushed since, which is exactly what a release
    /// exists to prevent.
    async fn restore(&self, project: &Project, service: &Service) -> bool {
        let current = releases::of_service(&self.database, &service.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|release| release.deployed_at.is_some());

        match current {
            Some(release) => {
                self.deploy_release(service, &release).await;
                true
            }
            None => self.deploy(project, service).await.is_ok(),
        }
    }

    /// Deploy one specific release.
    ///
    /// The image is the release's *digest*, not the tag it arrived
    /// under: a tag moves, and a deployment that resolved one again at
    /// run time would not be the release anybody chose. This is also
    /// what makes a rollback a rollback.
    pub async fn deploy_release(&self, service: &Service, release: &Release) {
        let Ok(Some(project)) = projects::find(&self.database, &service.project_id).await else {
            tracing::warn!(service = %service.slug, "no project for this service");
            return;
        };

        // A copy of the row pointed at the pinned reference. The
        // service's own `image` keeps naming the tag it watches —
        // changing it here would make the next push look like it was
        // for a different repository.
        let pinned = Service {
            image: release.pinned(),
            ..service.clone()
        };

        if self.deploy(&project, &pinned).await.is_ok() {
            if let Err(error) = releases::mark_deployed(&self.database, &release.id).await {
                tracing::error!(%error, "could not record which release is running");
            }
        }
    }

    /// Stop a service and take it off the network.
    ///
    /// Records `stopped` as the intent, so a reconcile does not start
    /// it again ten seconds later.
    /// Stop one copy and take it off the network, leaving the rest of
    /// the service alone.
    ///
    /// Without the desired state, deliberately: that is the *service's*
    /// intent, and one copy going away is not the service being
    /// stopped. Used when a node is told it no longer runs a slot — the
    /// others it holds keep running.
    pub async fn stop_replica(
        &self,
        project: &Project,
        service: &Service,
        replica: &Replica,
    ) -> DeployResult<()> {
        let id = replica.container_id(&project.slug, &service.slug);
        let client = Containerd::connect().await?;

        containers::stop(&client, &id, STOP_GRACE).await?;
        containers::remove(&client, &id).await?;
        if let Some(net) = self.network_of(project).await {
            network::detach(&net, &id).await;
        }
        replicas::set_address(&self.database, &replica.id, None).await?;
        self.sync_routes().await;

        tracing::info!(service = %service.slug, slot = replica.slot, "stopped one copy");
        Ok(())
    }

    pub async fn stop(&self, project: &Project, service: &Service) -> DeployResult<()> {
        // The intent first, so a reconcile arriving mid-stop does not
        // start back what is being taken down.
        services::set_desired_state(&self.database, &service.id, DesiredState::Stopped).await?;

        let client = Containerd::connect().await?;
        let net = self.network_of(project).await;

        for replica in self.mine(service).await? {
            let id = replica.container_id(&project.slug, &service.slug);
            containers::stop(&client, &id, STOP_GRACE).await?;
            containers::remove(&client, &id).await?;

            if let Some(net) = &net {
                network::detach(net, &id).await;
            }
            replicas::set_address(&self.database, &replica.id, None).await?;
            replicas::set_last_error(&self.database, &replica.id, None).await?;
        }
        self.sync_routes().await;

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

    /// A memory reading for this node, with the containers named.
    ///
    /// The container pids come from containerd rather than from the
    /// process table: a pid alone cannot say which service it is, and
    /// the console's whole point here is attribution.
    pub async fn memory(&self) -> crate::node::memory::Snapshot {
        let mut pids = std::collections::BTreeMap::new();

        if let Ok(client) = Containerd::connect().await {
            let services = services::all(&self.database, None)
                .await
                .unwrap_or_default();
            let projects = projects::all(&self.database).await.unwrap_or_default();

            for service in services {
                let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
                    continue;
                };
                let id = service.container_id(&project.slug);
                if let Ok(Some(status)) = containers::status(&client, &id).await {
                    if status.running() {
                        pids.insert(id, status.pid);
                    }
                }
            }
        }

        crate::node::memory::read(&pids)
    }

    /// What containerd says about one copy right now.
    pub async fn observe(
        &self,
        project: &Project,
        service: &Service,
        replica: &Replica,
    ) -> Observed {
        let client = match Containerd::connect().await {
            Ok(client) => client,
            Err(error) => return Observed::Unknown(error.to_string()),
        };

        let id = replica.container_id(&project.slug, &service.slug);
        match containers::status(&client, &id).await {
            Ok(Some(status)) if status.running() => Observed::Running {
                pid: status.pid,
                address: replica.address.clone(),
            },
            Ok(Some(TaskStatus { exit_code, .. })) => Observed::Stopped { exit_code },
            Ok(None) => Observed::Absent,
            Err(error) => Observed::Unknown(error.to_string()),
        }
    }

    /// What containerd says about the copy of this service that runs
    /// here, when one does.
    ///
    /// For a page that shows a service as one thing, which every page
    /// still does — the replicas each get their own line when the
    /// service page learns to place them. A service with nothing here
    /// reads as absent, which is true: there is no container on this
    /// node to ask about.
    pub async fn observe_service(&self, project: &Project, service: &Service) -> Observed {
        match self.mine(service).await {
            Ok(mine) => match mine.first() {
                Some(replica) => self.observe(project, service, replica).await,
                None => Observed::Absent,
            },
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
        // What should be up *here*, whoever placed it — a replica on
        // another node is not this one's to start, and an evicted one
        // is not either.
        let mine = replicas::here(&self.database).await?;
        let mut started = 0;

        for replica in mine {
            let Some(service) = services.iter().find(|s| s.id == replica.service_id) else {
                continue;
            };
            if service.desired_state != DesiredState::Running {
                continue;
            }
            let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
                continue;
            };

            match self.observe(project, service, &replica).await {
                Observed::Running { .. } => {}
                Observed::Unknown(error) => {
                    // Reconciling against a runtime that cannot answer
                    // would redeploy everything on the node because
                    // the socket was busy for a moment.
                    tracing::warn!(service = %service.slug, %error, "skipped: cannot ask containerd");
                }
                Observed::Absent | Observed::Stopped { .. } => {
                    tracing::info!(
                        service = %service.slug,
                        slot = replica.slot,
                        "reconciling: should be running"
                    );
                    if self.restore(project, service).await {
                        started += 1;
                    }
                }
            }
        }

        // Always, not only when something started: a node whose
        // containers all survived still needs its routes built, and
        // the control-plane rows are written here too.
        self.sync_routes().await;

        if started > 0 {
            tracing::info!(started, "reconciled");
        }
        Ok(started)
    }

    /// This node's own address on the overlay, if it is on one.
    async fn overlay_address(&self) -> Option<String> {
        crate::network::me(&self.database)
            .await
            .ok()
            .flatten()
            .and_then(|me| me.overlay_ip)
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

/// The port handed to the container as `PORT`.
///
/// A service can declare several; the one an application should bind
/// by default is the one that serves the site, and failing that the
/// lowest it declared. Guessing wrong is harmless — `PORT` is a hint
/// most runtimes read and none require — but guessing the *admin*
/// port of a service that also serves a site would be the one wrong
/// answer that looks right.
fn primary_port(ports: &[Port]) -> Option<u16> {
    let with_hostname = ports
        .iter()
        .filter(|port| port.hostname.is_some())
        .map(|port| port.container_port)
        .min();
    with_hostname.or_else(|| ports.iter().map(|port| port.container_port).min())
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

    fn port(container_port: u16, hostname: Option<&str>) -> Port {
        Port {
            id: format!("prt-{container_port}"),
            service_id: "svc-1".into(),
            container_port,
            host_port: None,
            hostname: hostname.map(str::to_string),
        }
    }

    /// The site's port, not the admin one — and the lowest declared
    /// when nothing serves a site.
    #[test]
    fn the_port_a_container_is_told_to_bind_is_the_one_that_serves() {
        assert_eq!(
            primary_port(&[port(9000, None), port(3000, Some("app.example.com"))]),
            Some(3000)
        );
        assert_eq!(
            primary_port(&[port(9000, None), port(3000, None)]),
            Some(3000)
        );
        assert_eq!(primary_port(&[]), None);
    }

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
