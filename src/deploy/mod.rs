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

pub mod database;
pub mod dns;
pub mod hosts;
pub mod jobs;
pub mod logs;
pub mod routing;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::platform::databases;
use crate::platform::ports::{self, Port};
use crate::platform::postgres;
use crate::platform::projects::Project;
use crate::platform::releases::{self, Release};
use crate::platform::replicas::{self, Replica};
use crate::platform::services::{self, DesiredState, Service};
use crate::platform::volumes::{self, Volume};
use crate::platform::{projects, PlatformError};
use crate::runtime::client::Containerd;
use crate::runtime::containers::{self, TaskStatus};
use crate::runtime::network::{self, PortMapping, ProjectNetwork};
use crate::runtime::spec::{BindMount, ContainerRequest};

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("containerd: {0}")]
    Runtime(#[from] crate::runtime::client::ClientError),
    #[error("network: {0}")]
    Network(#[from] network::NetworkError),
    /// The overlay's, not the bridge's. Reading which node holds a
    /// copy of a database is a question about the network of nodes.
    #[error("{0}")]
    Nodes(#[from] crate::network::NetworkError),
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

/// What an instruction to a node holding copies says about the placement.
///
/// Two instructions, not a flag: `slots: []` means "this service is not
/// yours to run any more" — the far node stops what it holds and deletes
/// its own rows — which is exactly right for a deletion and data loss for
/// a stop. Naming them keeps the empty vector from being something a
/// caller passes by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holding {
    /// What that node holds, and whether the service is meant to run.
    AsPlaced,
    /// Nothing.
    LetGo,
}

/// What a node has of one resource, and what is already spoken for.
#[derive(Debug, Clone, Copy)]
pub struct Room {
    /// What may be promised: the machine, less what the node keeps.
    pub allocatable: u64,
    /// Whether the machine's size could be read at all.
    pub known: bool,
    /// The sum of every ceiling on every copy here, this one included.
    pub committed: u64,
    /// What this service already holds, per copy.
    pub already: u64,
}

/// Whether one more promise this size can be kept, and why not.
///
/// Pure, and separate from the readings, because this is the part
/// somebody would argue with — and because a test for it should not
/// need a machine with a particular amount of memory. The first version
/// was not, and it refused *everything* on any machine whose total it
/// could not read: `room_for` ran on a laptop with no `/proc/meminfo`,
/// `allocatable` came out nought, and a node that cannot measure itself
/// became a node that will not let you set a limit.
///
/// **What is not known is not enforced.** A reserve you cannot measure
/// is not one you can hold back, and refusing on the strength of a
/// reading that failed is a rule made of a missing number.
///
/// `already` is subtracted before the sum, or a ceiling would be refused
/// by its own current value — a form that lets somebody set a number
/// once and never change their mind, which is worse than no check
/// because it looks like a rule.
pub fn fits(room: Room, wanted: u64, copies: u64, say: impl Fn(u64) -> String) -> Option<String> {
    if !room.known {
        return None;
    }
    let others = room
        .committed
        .saturating_sub(room.already.saturating_mul(copies));
    let after = others.saturating_add(wanted.saturating_mul(copies));
    if after <= room.allocatable {
        return None;
    }
    Some(format!(
        "this node has {} to promise and {} is already promised; {} × {copies} does not fit",
        say(room.allocatable),
        say(others),
        say(wanted),
    ))
}

/// What this node has promised to the copies it runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Committed {
    /// Bytes, summed over every copy here with a ceiling.
    pub memory: u64,
    /// Millicores, likewise.
    pub cpu: u32,
    /// How many copies have no memory ceiling at all.
    ///
    /// Reported rather than folded in, because there is no honest number
    /// to fold: a container with no limit may take everything, and a sum
    /// that silently omitted them would be a figure an operator trusts
    /// for a decision it cannot support.
    pub unbounded: usize,
}

/// Where a container's published ports are written, so that something
/// other than the deployment can ask what they were.
const PORTS_LABEL: &str = "wabot.ports";

/// And whether it was started archiving its write-ahead log.
///
/// The same idea as the ports, for the same reason: `archive_mode` is a
/// postmaster setting written into the arguments, so it changes only at
/// a deployment — and nothing would cause one. Turning the switch on
/// left a page saying "on" over a database that was not archiving, which
/// is worse than a switch that does nothing.
const ARCHIVING_LABEL: &str = "wabot.archiving";

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

    /// What this node is configured with — the data directory, mainly, for
    /// a caller that has the deployer and no config of its own.
    pub fn config(&self) -> &crate::config::Config {
        &self.config
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

        // And the names *inside* the containers, which answer the same
        // question one layer down. Here rather than in the deploy path
        // because every caller of this is something that moved a
        // container, and a container that moved is a name that points
        // somewhere else.
        self.sync_hosts().await;
    }

    /// Rewrite every local container's `/etc/hosts` from the rows.
    ///
    /// Every one of them, not only the service that changed: a
    /// deployment changes what its *neighbours* can reach, and a file
    /// that only the moved container had rewritten would leave the rest
    /// of the project pointing at where it used to be.
    ///
    /// In place, so a running container sees it at once — see
    /// `deploy::hosts`. That is the whole reason this is worth doing on
    /// every change rather than only at start.
    pub(crate) async fn sync_hosts(&self) {
        if let Err(error) = self.write_hosts().await {
            tracing::error!(%error, "could not update the names inside containers");
        }
    }

    async fn write_hosts(&self) -> DeployResult<()> {
        let domain = crate::node::settings::domain(&self.database, &self.config).await;
        let data_dir = &self.config.node.data_dir;

        for project in projects::all(&self.database).await? {
            let services = services::all(&self.database, Some(&project.id)).await?;

            // What each copy answers on. The reserved address when there
            // is one — a database's, which does not move — and otherwise
            // where it was last seen.
            let index = projects::ensure_network_index(&self.database, &project.id).await?;
            let net = ProjectNetwork::new(index)?;
            let mut addresses = std::collections::BTreeMap::new();
            let mut named = Vec::new();

            for service in &services {
                let row = match service.kind.is_managed() {
                    true => databases::of_service(&self.database, &service.id).await?,
                    false => None,
                };
                // The owner's domain, when this is somebody's copy. A
                // qualified name built from *this* machine's domain is
                // one no client would write — the database is not this
                // node's and neither is its name.
                let suffix = row
                    .as_ref()
                    .and_then(|r| r.owner_domain.clone())
                    .or_else(|| domain.clone());
                // The name itself, not the domain to build one from: a
                // database's is the operator's to choose and may sit under
                // a domain that has nothing to do with this node's.
                let qualified = match row.is_some() {
                    true => {
                        databases::qualified_name(
                            &self.database,
                            service,
                            &project.slug,
                            suffix.as_deref(),
                        )
                        .await?
                    }
                    // An ordinary service keeps the derived name it has
                    // always had in here. Its own hostname is what an edge
                    // serves, which is a different question from what the
                    // containers beside it resolve.
                    false => suffix
                        .as_ref()
                        .map(|domain| format!("{}.{}.{domain}", service.slug, project.slug)),
                };
                named.push((
                    service.slug.clone(),
                    row.as_ref().map(|r| r.primary_slot),
                    qualified,
                ));

                for replica in replicas::of_service(&self.database, &service.id).await? {
                    if !replica.is_here() || replica.evicted() {
                        continue;
                    }
                    let address = match replica.reserved_host {
                        Some(host) => Some(net.reserved_address(host)?.to_string()),
                        None => replica.address.clone(),
                    };
                    if let Some(address) = address {
                        addresses.insert((service.slug.clone(), replica.slot), address);
                    }
                }
            }

            for service in &services {
                for replica in replicas::of_service(&self.database, &service.id).await? {
                    if !replica.is_here() || replica.evicted() {
                        continue;
                    }
                    let id = replica.container_id(&project.slug, &service.slug);
                    // Per reader, because the read pool is ordered for
                    // whoever is going to read it.
                    let entries = hosts::entries_for(&named, &addresses, &project.slug, &id);
                    hosts::write(data_dir, &id, &entries)?;
                }
            }
        }
        Ok(())
    }

    /// Start (or restart) every copy of a service that runs here.
    ///
    /// The service is what to run; a replica is one running copy of it.
    /// Returns the address of the first one, which is what a caller
    /// that used to get "the" address wants — and what routing used to
    /// read before it learned to gather them.
    ///
    /// `None` when no copy of it runs **here**, which is a service placed
    /// entirely on other nodes. That used to be a refusal — "has no
    /// replica on this node" — and it was wrong in the way a page shows:
    /// the service exists, it runs, and the one thing its owner could not
    /// do was start it again after stopping it, because the only door
    /// refused before reaching the nodes that hold it.
    pub async fn deploy(
        &self,
        project: &Project,
        service: &Service,
    ) -> DeployResult<Option<Ipv4Addr>> {
        let mine = self.mine(service).await?;
        let Some(first) = mine.first().cloned() else {
            services::set_desired_state(&self.database, &service.id, DesiredState::Running).await?;
            self.tell_holders(project, service, Holding::AsPlaced).await;
            self.dispatch_standbys(service).await;
            tracing::info!(
                service = %service.slug,
                "no copy here: the nodes holding this service were told to run it"
            );
            return Ok(None);
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

        // And the nodes holding standbys are told where the primary
        // answers — *after* it deployed, which is when its overlay port
        // exists. An errand written when somebody clicked would have
        // carried an address that had not been assigned yet, which is
        // the whole reason this is recomputed rather than emitted.
        self.dispatch_standbys(service).await;
        // And the nodes holding plain copies are told the same thing: what
        // to run, and that it is meant to be running. This is also how a
        // new image reaches them — the errand is the whole of what that
        // node runs for the service.
        self.tell_holders(project, service, Holding::AsPlaced).await;

        match (first_address, failure) {
            (Some(address), _) => Ok(Some(address)),
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

        // A container that started and then died. The deployment
        // *worked* — containerd took it — so nothing above records
        // anything, and the console said `Stopped (exit 1)` with no
        // reason for an hour on a real node.
        //
        // Only for a managed engine, which is the case where the node
        // wrote the configuration and therefore owes an account of what
        // its own choices did.
        if result.is_ok() && service.kind.is_managed() {
            if let Some(reason) = self.died_saying(project, service, replica).await {
                replicas::set_last_error(&self.database, &replica.id, Some(&reason)).await?;
            }
        }
        result
    }

    /// What a container said, if it has already stopped saying it.
    ///
    /// Asked a moment after starting, which is when the configuration
    /// failures land: a `pg_hba.conf` Postgres will not parse, an
    /// argument it does not know, a data directory it refuses. A
    /// container that is still up after this answers `None` and is left
    /// alone — nothing here watches for a crash later, and the page
    /// reads the same log when it finds one stopped.
    async fn died_saying(
        &self,
        project: &Project,
        service: &Service,
        replica: &Replica,
    ) -> Option<String> {
        tokio::time::sleep(SETTLE).await;
        let Observed::Stopped { exit_code } = self.observe(project, service, replica).await else {
            return None;
        };

        let id = replica.container_id(&project.slug, &service.slug);
        Some(match logs::tail(&self.config.node.data_dir, &id, 1_000) {
            Some(said) => format!("it started and exited {exit_code}: {said}"),
            None => format!("it started and exited {exit_code}, saying nothing"),
        })
    }

    /// The copies of a service this node is the one running.
    async fn mine(&self, service: &Service) -> DeployResult<Vec<Replica>> {
        Ok(replicas::of_service(&self.database, &service.id)
            .await?
            .into_iter()
            .filter(|replica| replica.is_here() && !replica.evicted())
            .collect())
    }

    /// Every port this copy is published on, as the rows say it should
    /// be right now.
    ///
    /// Pulled out of the deployment so that **something else can ask the
    /// same question**: reconciliation compares this against what the
    /// running container was created with, and a comparison whose two
    /// halves are computed by different code is a comparison that finds
    /// differences nobody made.
    async fn published_ports(
        &self,
        service: &Service,
        replica: &Replica,
    ) -> DeployResult<Vec<PortMapping>> {
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
        //
        // And a managed database with a copy on another node, which is
        // the third reason and arrived with them: what has to reach it
        // is not an edge but a standby, dialling the primary to follow
        // it. Same port, same address, same reason it is not `0.0.0.0`.
        let dialled_by_a_copy_elsewhere = service.kind.is_managed()
            && replicas::of_service(&self.database, &service.id)
                .await?
                .iter()
                .any(|other| !other.is_here() && !other.evicted());

        let mut published = published;
        let reachable = ports
            .iter()
            .find(|port| port.hostname.is_some())
            .or_else(|| match service.is_ours() {
                true => None,
                false => ports.first(),
            })
            .or_else(|| match dialled_by_a_copy_elsewhere {
                true => ports.first(),
                false => None,
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

        Ok(published)
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
        let published = self.published_ports(service, replica).await?;
        // Before the container, and before the network is even torn
        // down on a failure: a bind of a directory that is not there
        // fails inside the shim, where the message is about a mount
        // rather than about the directory nobody created.
        let declared = volumes::of_service(&self.database, &service.id).await?;
        let mut mounts = self.mounts_for(&id, &declared)?;

        // The names of this project. The file has to exist before the
        // container does — a bind of something that is not there fails
        // inside the shim — and `sync_hosts` rewrites it in place from
        // then on, so a neighbour that appears later is reachable
        // without redeploying this.
        let hosts_file = hosts::path(&self.config.node.data_dir, &id);
        if !hosts_file.exists() {
            hosts::write(&self.config.node.data_dir, &id, &[])?;
        }
        if service.kind.is_managed() {
            // Inside the volume, and mounted writable: the one-shot
            // `chown` above works on this same directory, and a
            // read-only mount would refuse it.
            mounts.push(BindMount {
                source: database::tls_dir(&self.config.node.data_dir, &id),
                destination: postgres::TLS_DIR.to_string(),
                read_only: false,
            });
        }
        mounts.push(BindMount {
            source: hosts_file,
            destination: "/etc/hosts".to_string(),
            // The node writes it. A container editing its own would be
            // editing something the next change overwrites anyway.
            read_only: true,
        });

        // This node's certificate authority, in every container it starts.
        //
        // Because a database's certificate is signed by it, and a client
        // that cannot verify a certificate is a client using `require`
        // instead of `verify-full` — which is encryption without identity.
        // `docs/naming.md` decided this ("the node mounts `local-ca.crt`
        // into every container it starts, so nothing in the image has to
        // know anything") and it had never been built: a connection string
        // naming `sslrootcert` would have pointed at a file that was not
        // there.
        //
        // Into the same directory a managed engine's files arrive in, so
        // there is one destination and one mount. Two binds on `/etc/wabot`
        // would be a race about which one wins.
        match crate::edge::certs::ca_certificate_pem(&self.database).await {
            Ok(pem) => {
                let dir = database::config_dir(&self.config.node.data_dir, &id);
                std::fs::create_dir_all(&dir)?;
                std::fs::write(dir.join("ca.crt"), pem)?;
                // A database mounts this directory itself, below.
                if !service.kind.is_managed() {
                    mounts.push(BindMount {
                        source: dir,
                        destination: postgres::CONFIG_MOUNT.to_string(),
                        read_only: true,
                    });
                }
            }
            // Not fatal. A container that cannot verify is a container
            // using `sslmode=require`, which is what every one of them did
            // until now.
            Err(error) => tracing::warn!(%error, "could not place the node's authority"),
        }

        // What only a managed engine needs: its generated files, its
        // tuning, its credentials and its role.
        let prepared = self.prepare_engine(project, service, replica, &net).await?;
        // Written on the container below, so reconciliation can tell a
        // database started before the switch moved from one started
        // after it.
        let archiving = crate::node::settings::archiving(&self.database).await;
        if let Some(prepared) = &prepared {
            mounts.extend(prepared.mounts.iter().cloned());
        }

        // An address that will not move, for a container whose address
        // somebody wrote into a connection string. `None` for
        // everything else, which is every service that is not a
        // database — see `runtime::network::RESERVED_HOSTS`.
        let wanted = replica
            .reserved_host
            .map(|host| net.reserved_address(host))
            .transpose()?;

        let address = network::attach(&net, &id, &published, wanted).await?;

        self.write_resolv_conf()?;
        let mut env: Vec<(String, String)> = service
            .env
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        // After the operator's, so the engine's own settings win. A
        // managed database's environment is the node's to write, and a
        // `POSTGRES_PASSWORD` from anywhere else is one the console
        // would then be showing the wrong value for.
        if let Some(prepared) = &prepared {
            env.extend(prepared.env.iter().cloned());
        }

        let request = ContainerRequest {
            command: Vec::new(),
            args: prepared
                .as_ref()
                .map(|prepared| prepared.args.clone())
                .unwrap_or_default(),
            env,
            port: primary_port(&ports),
            network_ns: Some(network::netns_path(&id)),
            resolv_conf: Some(self.resolv_conf.clone()),
            mounts,
            memory_limit: service.memory_limit,
            cpu_millicores: service.cpu_millicores,
            // What this container is being published on, written on it.
            // Reconciliation reads it back and compares — the one thing
            // it never checked was whether the ports it opened are the
            // ports the rows now ask for, so a mapping could be right
            // in the database and absent on the machine with nothing
            // saying so.
            labels: std::collections::BTreeMap::from([
                (PORTS_LABEL.to_string(), network::render(&published)),
                (
                    ARCHIVING_LABEL.to_string(),
                    match prepared.is_some() {
                        true => archiving.to_string(),
                        false => String::new(),
                    },
                ),
            ]),
            shm_size: prepared.as_ref().and_then(|prepared| prepared.shm_size),
        };

        // What this node presents to the registry the image lives on,
        // if it has been given anything. `None` is the ordinary case —
        // an image from a registry that serves anybody.
        let credential =
            crate::platform::registry_credentials::for_reference(&self.database, &service.image)
                .await;

        // The certificate, before the server that reads it. The names
        // it covers are the ones a client can connect to and still have
        // `verify-full` pass: the short ones inside the project, and the
        // qualified one that is also the public name when there is a
        // domain.
        if prepared.is_some() {
            let mut names = vec![
                service.slug.clone(),
                format!("{}.{}", service.slug, project.slug),
            ];
            if let Some(domain) = crate::node::settings::domain(&self.database, &self.config).await
            {
                // First, so it is the certificate's common name: it is
                // the one a public authority can sign, and the one a
                // client outside this node would use.
                names.insert(0, format!("{}.{}.{domain}", service.slug, project.slug));
            }
            self.place_certificate(&client, service, &names, &id)
                .await?;
        }

        // A copy being restored gets its base backup unpacked first, for
        // the same reason a standby gets seeded: the container is a
        // server that expects to find a data directory, and this is
        // where it comes from. Skipped once the volume holds one —
        // recovery is Postgres's business from there, and it deletes
        // `recovery.signal` itself when it finishes.
        if let Some(prepared) = &prepared {
            if let Some(restoring) = &prepared.restoring {
                if !database::seeded(&self.config.node.data_dir, &id) {
                    self.unpack_base(&client, service, &id, restoring).await?;
                }
            }
        }

        // A standby with an empty volume gets the primary copied into
        // it first. Before the container, because the container is the
        // server that expects to find a data directory there — and
        // before the network, because the seed has none of its own.
        if let Some(prepared) = &prepared {
            // A standby whose ceiling has come down since it was seeded
            // cannot start at the new one — the log it still has to
            // replay records the primary's old value, and Postgres
            // refuses rather than replay it with less. The directory is
            // finished; a fresh copy of the primary, which is already
            // at the new rung, has no such record in it. See
            // `database::outgrown`.
            if prepared.role == postgres::Role::Standby
                && database::outgrown(&self.config.node.data_dir, &id, prepared.max_connections)
            {
                tracing::warn!(
                    service = %service.slug,
                    slot = replica.slot,
                    max_connections = prepared.max_connections,
                    "this copy's ceiling came down; copying the primary in again"
                );
                database::discard_standby_data(&self.config.node.data_dir, &id)
                    .map_err(|error| DeployError::Refused(error.to_string()))?;
            }
            if prepared.role == postgres::Role::Standby
                && !database::seeded(&self.config.node.data_dir, &id)
            {
                let row = databases::of_service(&self.database, &service.id)
                    .await?
                    .ok_or_else(|| DeployError::Refused("no engine row".into()))?;
                let endpoint = prepared.primary_endpoint.clone().ok_or_else(|| {
                    DeployError::Refused(format!(
                        "the primary of {} has no address this node can reach yet",
                        service.slug
                    ))
                })?;
                self.seed_standby(&client, service, &row, &id, endpoint, replica.slot)
                    .await?;
            }
            // From the rows, every time. A standby that was promoted
            // would have had this deleted by Postgres, and recreating it
            // is how a promotion gets undone — which is why promoting
            // has to move `primary_slot` rather than touch the volume.
            if prepared.role == postgres::Role::Standby {
                database::write_standby_signal(&self.config.node.data_dir, &id)?;
                // After the seed, so a directory that has just arrived
                // from the primary is recorded at the value it is about
                // to run at.
                database::record_ceiling(&self.config.node.data_dir, &id, prepared.max_connections);
            }
        }

        // The log first, emptied: what a container says while failing
        // is the only thing that explains it, and containerd discards
        // it unless it is given somewhere to put it.
        //
        // Every container, not only a managed engine. That restriction
        // was about the disk — nothing bounded the file — and the page
        // it left behind told the owner of a plain container that its
        // output had not been kept and to deploy it again, which could
        // never work. `logs::trim_all` is the bound now, and it belongs
        // to the disk rather than to who wrote the configuration.
        let log = logs::prepare(&self.config.node.data_dir, &id).ok();

        match containers::run(
            &client,
            &id,
            &service.image,
            &request,
            credential.as_ref(),
            log.as_deref(),
        )
        .await
        {
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

    /// Whether the running container was started with what the rows now
    /// say.
    ///
    /// Both halves come from `published_ports` and `network::render` —
    /// the same two functions the deployment used — so a difference here
    /// is a difference in the rows and never in the spelling.
    ///
    /// True when there is nothing to compare against: a container from
    /// before this was written carries no label, and redeploying every
    /// container on the node once because of that would be this check
    /// doing more damage than the drift it looks for.
    async fn started_as_asked(
        &self,
        project: &Project,
        service: &Service,
        replica: &Replica,
    ) -> bool {
        let id = replica.container_id(&project.slug, &service.slug);
        let Ok(client) = Containerd::connect().await else {
            return true;
        };
        let labels = match containers::labels(&client, &id).await {
            Ok(Some(labels)) => labels,
            // No container, or a runtime that will not answer: neither
            // is this question's to report on.
            Ok(None) | Err(_) => return true,
        };

        // Archiving first, because it is the cheaper question and the
        // one an upgrade changes: the default flipped once pruning
        // existed, and every database already running was started
        // without it.
        if service.kind.is_managed() {
            if let Some(applied) = labels.get(ARCHIVING_LABEL) {
                let wanted = crate::node::settings::archiving(&self.database).await;
                if applied != &wanted.to_string() {
                    return false;
                }
            }
        }

        let Some(applied) = labels.get(PORTS_LABEL) else {
            return true;
        };
        match self.published_ports(service, replica).await {
            Ok(wanted) => applied == &network::render(&wanted),
            // The rows could not be read, which is not the container's
            // fault and not a reason to replace it.
            Err(_) => true,
        }
    }

    /// What this node has promised, and what it has left.
    ///
    /// **A ceiling is also a reservation.** There is deliberately no
    /// second number for a request — see `migrations/0038_cpu_limit.sql`
    /// — so what a service may take is what the node counts against
    /// itself when deciding whether it can take another.
    ///
    /// Counted per *replica* rather than per service: a service with
    /// three copies here costs three times its ceiling, which is what it
    /// actually costs. And only the copies here — a copy elsewhere is
    /// that node's promise to keep.
    ///
    /// A service with no ceiling counts as nothing, which is the honest
    /// answer and an uncomfortable one: it can take everything and the
    /// arithmetic cannot see it. The page says so rather than quietly
    /// pretending the sum means more than it does.
    pub async fn committed(&self) -> Committed {
        let (Ok(services), Ok(mine)) = (
            services::all(&self.database, None).await,
            replicas::here(&self.database).await,
        ) else {
            return Committed::default();
        };

        let mut committed = Committed::default();
        for replica in mine.iter().filter(|replica| !replica.evicted()) {
            let Some(service) = services.iter().find(|s| s.id == replica.service_id) else {
                continue;
            };
            match service.memory_limit {
                Some(bytes) => committed.memory += bytes,
                None => committed.unbounded += 1,
            }
            if let Some(millicores) = service.cpu_millicores {
                committed.cpu += millicores;
            }
        }
        committed
    }

    /// Whether this node can keep a promise this size, or why not.
    ///
    /// `None` is yes. The reason is a sentence somebody can act on,
    /// which is this project's rule about errors and matters more here
    /// than usual: "refused" without a number is a form somebody fights
    /// with by trying smaller values.
    ///
    /// This half gathers; [`fits`] decides. The judgement is worth
    /// having apart from the four readings it needs — it is where the
    /// arithmetic somebody would argue with lives, and a test for it
    /// should not need a machine with a particular amount of memory.
    pub async fn room_for(
        &self,
        service: &Service,
        memory: Option<u64>,
        cpu: Option<u32>,
    ) -> Option<String> {
        let mine = replicas::here(&self.database).await.ok()?;
        let copies = mine
            .iter()
            .filter(|replica| replica.service_id == service.id && !replica.evicted())
            .count()
            .max(1) as u64;

        let committed = self.committed().await;
        let total = self.memory().await.total;

        if let Some(bytes) = memory {
            let refusal = fits(
                Room {
                    allocatable: crate::platform::presets::allocatable_memory(total),
                    known: total > 0,
                    committed: committed.memory,
                    already: service.memory_limit.unwrap_or(0),
                },
                bytes,
                copies,
                crate::node::memory::human,
            );
            if refusal.is_some() {
                return refusal;
            }
        }

        if let Some(millicores) = cpu {
            let cores = crate::node::cpu::allocatable_millicores();
            return fits(
                Room {
                    allocatable: u64::from(cores),
                    known: cores > 0,
                    committed: u64::from(committed.cpu),
                    already: u64::from(service.cpu_millicores.unwrap_or(0)),
                },
                u64::from(millicores),
                copies,
                |millicores| {
                    crate::platform::presets::cpu_label(
                        u32::try_from(millicores).unwrap_or(u32::MAX),
                    )
                },
            );
        }
        None
    }

    /// The copies of this service that are not answering, by replica id.
    ///
    /// The edge probes upstream *addresses*; a page shows replica
    /// *rows*. This is the join, and it derives nothing of its own —
    /// `routing::upstreams_of` is the same function that told the edge
    /// where to send, so the address asked about is the address probed.
    ///
    /// Empty when this node routes nothing for the service, which is
    /// the ordinary case for a node that owns it and had it served
    /// elsewhere: health is known by the node doing the proxying, and
    /// this one has no opinion to offer.
    pub async fn not_answering(&self, service: &Service) -> Vec<String> {
        let Some(routes) = &self.routes else {
            return Vec::new();
        };
        let health = routes.health();
        let down: Vec<std::net::SocketAddr> = health
            .down()
            .into_iter()
            .map(|(address, _)| address)
            .collect();
        if down.is_empty() {
            return Vec::new();
        }

        let (Ok(here), Ok(elsewhere), Ok(ports)) = (
            replicas::here(&self.database).await,
            replicas::elsewhere(&self.database).await,
            ports::of_service(&self.database, &service.id).await,
        ) else {
            return Vec::new();
        };
        let nodes = crate::network::all(&self.database)
            .await
            .unwrap_or_default();

        let mut failing = Vec::new();
        for port in &ports {
            for (replica, address) in crate::deploy::routing::upstreams_of(
                &here,
                &elsewhere,
                &nodes,
                &service.id,
                port.container_port,
            ) {
                if down.contains(&address) && !failing.contains(&replica) {
                    failing.push(replica);
                }
            }
        }
        failing
    }

    /// Everything on this node's disk that no copy claims.
    ///
    /// A copy leaves four things behind and until now only one of them
    /// was ever looked for. `volumes::orphans` has reported unclaimed
    /// data since databases existed, and the configuration written for
    /// a container, the names it was given and what it said were in no
    /// list at all — so a node that has moved copies around keeps a file
    /// per container that ever ran on it, and nothing says so. There is
    /// one on the Ubuntu test node for a replica that moved to Alpine.
    ///
    /// Reported, never removed, which is the rule the volume half
    /// already followed: a directory whose rows are missing for a reason
    /// nobody has understood yet is one somebody can still recover from.
    /// The three small ones hold nothing anybody wants back — they are
    /// rebuilt from the rows — but a thing this node cannot explain is
    /// not a thing it should delete on sight.
    pub fn leftovers(data_dir: &std::path::Path, live: &[String]) -> Vec<(&'static str, PathBuf)> {
        let mut found: Vec<(&'static str, PathBuf)> = volumes::orphans(data_dir, live)
            .into_iter()
            .map(|path| ("data", path))
            .collect();

        // The other three are named after the container id exactly —
        // a directory for the generated configuration, a file for the
        // names, a file for the output with `.log` after it.
        for (kind, directory, suffix) in [
            ("config", "config", ""),
            ("hosts", "hosts", ""),
            ("log", "logs", ".log"),
        ] {
            let Ok(entries) = std::fs::read_dir(data_dir.join(directory)) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let Some(id) = name.strip_suffix(suffix) else {
                    continue;
                };
                if !live.iter().any(|claimed| claimed == id) {
                    found.push((kind, entry.path()));
                }
            }
        }
        found.sort_by(|left, right| left.1.cmp(&right.1));
        found
    }

    /// Take one copy away for good: stop it, let go of what it held,
    /// and drop its row.
    ///
    /// **A standby's data directory goes with the row.** It holds a copy
    /// of the primary and nothing of its own, and a slot filled again
    /// later gets the same container id — so the directory left behind
    /// is adopted rather than seeded, and it fails in two ways that both
    /// read as the database being broken: the parameters recorded in the
    /// log it has not replayed (see `database::outgrown`), and a
    /// replication slot dropped meanwhile, whose write-ahead log the
    /// primary has long since recycled.
    ///
    /// Reported by Jorge, who did the reasonable thing with a standby
    /// that would not start — took the copies down to one and back to
    /// two — and got the identical failure back, because the second
    /// standby was the first one's directory.
    ///
    /// Never the primary's, and never a plain service's volume:
    /// `volumes::discard` has exactly one other caller, a deletion
    /// somebody confirmed. A *stop* keeps everything — this is removal.
    pub async fn forget_replica(
        &self,
        project: &Project,
        service: &Service,
        replica: &Replica,
    ) -> DeployResult<()> {
        if let Err(error) = self.stop_replica(project, service, replica).await {
            // Reported and carried on from: the row is going either way,
            // and a container this node could not reach must not keep it
            // listed as something it runs.
            tracing::warn!(slot = replica.slot, %error, "stopping a copy that was dropped");
        }

        let id = replica.container_id(&project.slug, &service.slug);
        if service.kind.is_managed() {
            let primary = databases::of_service(&self.database, &service.id)
                .await?
                .map(|row| row.primary_slot);
            if primary.is_some_and(|slot| slot != replica.slot) {
                if let Err(error) = volumes::discard(&self.config.node.data_dir, &id) {
                    tracing::warn!(container = %id, %error, "discarding a standby's data");
                }
            }
        }

        // And the three small things a copy leaves beside its data: the
        // configuration written for it, the names it was given, and what
        // it said. None is worth keeping — every one is rebuilt from the
        // rows at the next deployment — and none of them is ever removed
        // by anything else, because the only other caller of these is a
        // deletion of the whole service. So a node that has moved copies
        // around keeps a file per container that ever ran on it: there
        // is one on the Ubuntu node right now for a replica that went to
        // Alpine.
        database::discard(&self.config.node.data_dir, &id);
        hosts::discard(&self.config.node.data_dir, &id);
        logs::discard(&self.config.node.data_dir, &id);

        replicas::remove(&self.database, &replica.id).await?;
        Ok(())
    }

    pub async fn stop(&self, project: &Project, service: &Service) -> DeployResult<()> {
        // The intent first, so a reconcile arriving mid-stop does not
        // start back what is being taken down.
        services::set_desired_state(&self.database, &service.id, DesiredState::Stopped).await?;

        // The other machines next, and **before containerd** on purpose:
        // the instruction is the intent, so it belongs with it, and a node
        // whose own runtime will not answer must still be able to say
        // "stop" to the nodes running the rest of this service. Below this
        // line the first thing that fails returns.
        //
        // Both, because a service is told by one of them and a database by
        // the other, and each declines what is not its business. Leaving
        // the second one out is how a database Jorge stopped went on being
        // followed by a standby on another machine: `tell_holders` returns
        // at once for a managed kind, `dispatch_standbys` is what carries
        // its intent, and nothing here called it.
        self.tell_holders(project, service, Holding::AsPlaced).await;
        self.dispatch_standbys(service).await;

        self.stop_here(project, service).await?;
        tracing::info!(service = %service.slug, project = %project.slug, "stopped");
        Ok(())
    }

    /// Take down the copies that run on **this** machine.
    ///
    /// The half of a stop that has nothing to say to anybody else, split
    /// out because a deletion needs it and must not send a stop's
    /// instruction on the way: "stop and keep it" is the opposite of what
    /// a deletion means.
    async fn stop_here(&self, project: &Project, service: &Service) -> DeployResult<()> {
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
        Ok(())
    }

    /// Remove everything a deleted service leaves behind.
    ///
    /// Best effort by design: the row is going either way, and a
    /// container we could not reach must not keep the operator from
    /// deleting the service they asked to delete. What it must not do
    /// is fail *silently* — hence the logs.
    ///
    /// **The withdrawal goes first, and it is not a stop.** It is built
    /// from the rows, so it has to leave before they do — and a deletion
    /// is the one case with no convergent fallback: after the rows are
    /// gone there is nothing here to recompute from, so a copy this never
    /// reached would run on somewhere else with nobody left to contradict
    /// it. Every other instruction in this file gets a second chance at
    /// the next boot. This one does not.
    pub async fn tear_down(&self, project: &Project, service: &Service) {
        self.tell_holders(project, service, Holding::LetGo).await;
        if let Err(error) = crate::platform::databases::withdraw(
            &self.database,
            service,
            self.primary_overlay(service).await.ok().flatten(),
            crate::node::settings::domain(&self.database, &self.config).await,
        )
        .await
        {
            tracing::warn!(service = %service.slug, %error, "letting go of a standby elsewhere");
        }

        if let Err(error) = self.stop_here(project, service).await {
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

            // Every **replica**, not every service.
            //
            // This asked for `service.container_id`, which is slot 1's —
            // so a service with copies in slots 2 and 3 had one of them
            // counted and the memory page reported less than the machine
            // was using. Found on the test node, where a three-copy
            // service and a two-copy database contributed one container
            // each.
            for service in services {
                let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
                    continue;
                };
                for replica in replicas::of_service(&self.database, &service.id)
                    .await
                    .unwrap_or_default()
                {
                    if !replica.is_here() || replica.evicted() {
                        continue;
                    }
                    let id = replica.container_id(&project.slug, &service.slug);
                    if let Ok(Some(status)) = containers::status(&client, &id).await {
                        if status.running() {
                            pids.insert(id, status.pid);
                        }
                    }
                }
            }
        }

        crate::node::memory::read(&pids)
    }

    /// Which cgroup each running container is in, by container id.
    ///
    /// For the CPU reading, which needs `cpu.stat` and therefore the path
    /// rather than the pid. Read from `/proc/<pid>/cgroup` because that is
    /// the one source that cannot disagree with where the process actually
    /// is — a path built from the container id would be guessing at the
    /// runtime's naming, and crun's differs from runc's.
    pub async fn cgroups(&self) -> std::collections::BTreeMap<String, String> {
        let mut paths = std::collections::BTreeMap::new();
        let Ok(client) = Containerd::connect().await else {
            return paths;
        };
        let services = services::all(&self.database, None)
            .await
            .unwrap_or_default();
        let projects = projects::all(&self.database).await.unwrap_or_default();

        for service in services {
            let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
                continue;
            };
            for replica in replicas::of_service(&self.database, &service.id)
                .await
                .unwrap_or_default()
            {
                if !replica.is_here() || replica.evicted() {
                    continue;
                }
                let id = replica.container_id(&project.slug, &service.slug);
                if let Ok(Some(status)) = containers::status(&client, &id).await {
                    if !status.running() {
                        continue;
                    }
                    if let Some(path) = cgroup_of(status.pid) {
                        paths.insert(id, path);
                    }
                }
            }
        }
        paths
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
                // Running is not the same as running *as the rows say*.
                // A container is created with the ports it was given
                // written on it, and this is the one thing that asks
                // whether they are still the ports the rows ask for —
                // until now a published port that had been added,
                // removed or moved since the container started stayed
                // that way until something else happened to redeploy.
                Observed::Running { .. } => {
                    match self.started_as_asked(project, service, &replica).await {
                        true => {}
                        false => {
                            tracing::info!(
                                service = %service.slug,
                                slot = replica.slot,
                                "reconciling: it was started with something the rows no longer say"
                            );
                            if self.restore(project, service).await {
                                started += 1;
                            }
                        }
                    }
                }
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

        // And the copies this node no longer agrees to run, for the same
        // reason and by the same rule: the grant is the more recent
        // decision, and nothing else will come and act on it.
        self.evict_ungranted().await;

        // Names held for a node this one no longer serves, before the
        // routes are built rather than after: the point is that the
        // table the listener reads comes up without them.
        match crate::network::release_ungranted(&self.database).await {
            Ok(released) if !released.is_empty() => {
                tracing::info!(names = released.join(", "), "released names");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "could not release the names held for others"),
        }

        // Always, not only when something started: a node whose
        // containers all survived still needs its routes built, and
        // the control-plane rows are written here too.
        self.sync_routes().await;

        // And what the nodes holding standbys were told, for the same
        // reason. That errand is *derived* — from the primary's overlay
        // port, from this node's domain, from what the payload has room
        // to say — and until now nothing recomputed it except a
        // deployment. So a domain set after the fact, or a field added
        // by an upgrade, reached nobody: the copy went on answering to
        // a name built from the wrong domain, on a node that had no way
        // to learn better. Measured, on Alpine, after this very field
        // was added.
        //
        // `queue_if_changed` makes the pass free when the answer is the
        // same, which it is on almost every boot.
        for service in &services {
            self.dispatch_standbys(service).await;
            // The same pass for plain copies. A node that was unreachable
            // when somebody pressed stop is told here instead — `stop`
            // reaches a node that answers, and this reaches the one that
            // did not, which between them is the only way an instruction
            // arrives at a machine that was off.
            if let Some(project) = projects.iter().find(|p| p.id == service.project_id) {
                self.tell_holders(project, service, Holding::AsPlaced).await;
            }
        }

        if started > 0 {
            tracing::info!(started, "reconciled");
        }
        Ok(started)
    }

    /// Tell the nodes holding standbys where this database's primary
    /// answers.
    ///
    /// Reads the address the same way a standby *here* would: the
    /// primary's overlay port on this node. Nothing is sent while there
    /// is no address — a standby with nowhere to follow would come up
    /// as a primary of its own.
    /// The same, telling nodes that have just lost their last copy.
    ///
    /// Separate entry point rather than a default, because the set is only
    /// knowable by the caller that made the change: after the rows move,
    /// nothing points at the node that lost one.
    pub async fn dispatch_standbys_including(
        &self,
        service: &Service,
        also: &std::collections::BTreeSet<String>,
    ) {
        self.standbys(service, also).await;
    }

    async fn dispatch_standbys(&self, service: &Service) {
        self.standbys(service, &Default::default()).await;
    }

    async fn standbys(&self, service: &Service, also: &std::collections::BTreeSet<String>) {
        if !service.kind.is_managed() || !service.is_ours() {
            return;
        }
        let primary = match self.primary_overlay(service).await {
            Ok(primary) => primary,
            Err(error) => {
                tracing::warn!(service = %service.slug, %error, "could not read the primary");
                return;
            }
        };
        let domain = crate::node::settings::domain(&self.database, &self.config).await;
        if let Err(error) =
            databases::dispatch(&self.database, service, primary, domain, also).await
        {
            tracing::warn!(service = %service.slug, %error, "could not tell a node to hold a copy");
        }
    }

    /// Throw off what this node no longer agrees to run.
    ///
    /// **Because stopping a copy needs the same permission as placing
    /// one.** That is the right rule — the same consent both ways, so an
    /// authority cannot reach into a machine that has shut the door on
    /// it — and it leaves exactly one hole: a node that revokes `host`
    /// can no longer be told to stop what it is running, so without this
    /// those containers would run for ever. Which is the phase 8 lesson
    /// again: the withdrawing errand arrives only if the other node is
    /// still there, still knows and still reaches this one, and a node
    /// revoking a grant is often doing it because one of those stopped
    /// being true.
    ///
    /// So it is local and convergent, at boot, asking only about now —
    /// the shape `network::release_ungranted` already has for names.
    ///
    /// **Evicted, not stopped.** Somebody did throw this out; the row is
    /// the tombstone that says so, and the next report tells the node
    /// that placed it to stop asking. A copy merely stopped would be
    /// started again by whoever placed it, which is the argument all over
    /// again with extra steps.
    async fn evict_ungranted(&self) {
        let (services, projects) = match (
            services::all(&self.database, None).await,
            projects::all(&self.database).await,
        ) {
            (Ok(services), Ok(projects)) => (services, projects),
            _ => {
                tracing::warn!("could not read what this node is running for others");
                return;
            }
        };

        for service in services.iter().filter(|service| !service.is_ours()) {
            let Some(authority) = &service.origin_node_id else {
                continue;
            };
            // Holding somebody's data and running somebody's container
            // are different favours, and each is withdrawn on its own.
            let needed = match service.kind.is_managed() {
                true => crate::network::capability::Capability::Store,
                false => crate::network::capability::Capability::Host,
            };
            // Read through what this node provides *now*, so a switch
            // turned off withdraws what was granted of it — the switch is
            // the more recent decision.
            if crate::network::capability::granted_to(&self.database, authority)
                .await
                .contains(&needed)
            {
                continue;
            }
            let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
                continue;
            };
            let Ok(mine) = self.mine(service).await else {
                continue;
            };

            for replica in mine {
                if let Err(error) = self.stop_replica(project, service, &replica).await {
                    // Said out loud and then evicted anyway: the row has
                    // to stop claiming a copy this node is refusing to
                    // run, whether or not the container could be reached.
                    tracing::warn!(service = %service.slug, %error, "could not stop a copy");
                }
                let _ = replicas::set_last_error(
                    &self.database,
                    &replica.id,
                    Some("this node no longer agrees to run it"),
                )
                .await;
                if let Err(error) = replicas::evict(&self.database, &replica.id).await {
                    tracing::warn!(service = %service.slug, %error, "could not evict a copy");
                    continue;
                }
                tracing::info!(
                    service = %service.slug,
                    slot = replica.slot,
                    authority = %authority,
                    capability = needed.name(),
                    "threw off a copy this node no longer agrees to run"
                );
            }
        }
    }

    /// Tell every node holding a copy what to run for this service, and
    /// whether it is meant to be running at all.
    ///
    /// [`Holding::LetGo`] is the other instruction: nothing, which the far
    /// node reads as "this service is not yours to run any more" — it stops
    /// what it holds and deletes its own rows. That is right for a deletion
    /// and would be data loss for a stop, which is why the two are a named
    /// argument rather than an empty vector somebody passes by accident.
    ///
    /// **A stop has to travel.** `stop` took down the copies here and said
    /// nothing to the machines running the others, so a service the
    /// console showed as stopped went on serving traffic somewhere else.
    /// Found by Jorge on the test nodes, and it is the same shape as every
    /// other bug this network has had: derived state that nothing
    /// recomputes when the thing it derives from changes.
    ///
    /// Derived and queued only when it differs, like `dispatch_standbys` —
    /// so the pass costs nothing on the boots where nothing moved, and a
    /// node that was unreachable when somebody pressed stop is told the
    /// next time it is asked.
    ///
    /// A managed database is **not** dispatched here. What a standby needs
    /// is not what a container needs, and a `host` errand would recreate
    /// it as a plain container without its volume or its engine
    /// arguments; those travel on `Kind::Database`, which carries the same
    /// intent for the same reason.
    /// The same, telling a node that holds nothing of this service.
    ///
    /// The set exists for one reason: `by_node` is built from the rows, so
    /// a node with no copy left is not in it — and it is the one that has
    /// to be told it runs none of this now.
    pub async fn tell_holders_including(
        &self,
        project: &Project,
        service: &Service,
        also: &std::collections::BTreeSet<String>,
    ) {
        self.holders(project, service, Holding::AsPlaced, also)
            .await;
    }

    async fn tell_holders(&self, project: &Project, service: &Service, holding: Holding) {
        self.holders(project, service, holding, &Default::default())
            .await;
    }

    async fn holders(
        &self,
        project: &Project,
        service: &Service,
        holding: Holding,
        also: &std::collections::BTreeSet<String>,
    ) {
        if service.kind.is_managed() || !service.is_ours() {
            return;
        }
        let Some(registry) = crate::platform::registry_credentials::host_of(&service.image) else {
            return;
        };
        let (placements, ports, nodes) = match (
            replicas::of_service(&self.database, &service.id).await,
            crate::platform::ports::of_service(&self.database, &service.id).await,
            crate::network::all(&self.database).await,
        ) {
            (Ok(placements), Ok(ports), Ok(nodes)) => (placements, ports, nodes),
            _ => {
                tracing::warn!(service = %service.slug, "could not read who holds a copy");
                return;
            }
        };

        // Which node holds which slots. One instruction per node, naming
        // every copy it holds: an errand *is* the whole of what that node
        // runs for this service, which is what lets a copy be taken away
        // and the far side find out.
        let mut by_node: std::collections::BTreeMap<String, Vec<u32>> = Default::default();
        // Nodes to tell even with nothing to give them, which is how one
        // learns that its last copy went home.
        for node_id in also {
            by_node.entry(node_id.clone()).or_default();
        }
        for replica in placements.iter().filter(|replica| !replica.evicted()) {
            if let Some(node_id) = &replica.node_id {
                by_node
                    .entry(node_id.clone())
                    .or_default()
                    .push(replica.slot);
            }
        }

        // The intent comes from the row, never from the caller's copy of
        // it. `stop` writes the state and then calls this with the struct
        // it was handed, which still says `Running` — so the instruction
        // that travelled said "run it", and a stop reached the other node
        // as no change at all. The test below had quietly arranged the
        // fresh struct the code should have read for itself, which is how
        // it passed while the path was broken.
        let running = services::find(&self.database, &service.id)
            .await
            .ok()
            .flatten()
            .map_or(service.desired_state, |row| row.desired_state)
            == DesiredState::Running;

        let host = crate::network::capability::Capability::Host;
        for (node_id, mut slots) in by_node {
            slots.sort_unstable();
            let (slots, running) = match holding {
                Holding::AsPlaced => (slots, running),
                // Nothing left to hold, and nothing to run while it is
                // being taken away.
                Holding::LetGo => (Vec::new(), false),
            };
            // A node that has taken `host` away is not sent this, and not
            // because the stop is unwelcome: it would refuse it. Stopping
            // needs the same permission as placing — the far side checks
            // it either way — and what a node no longer consents to run is
            // for that node to throw off, which it does at its own boot
            // without asking anybody. See `evict_ungranted`.
            if !nodes
                .iter()
                .any(|node| node.id == node_id && node.allows.contains(&host))
            {
                tracing::debug!(node = %node_id, "skipped: it no longer runs containers for us");
                continue;
            }

            let payload = match serde_json::to_value(crate::network::errand::Host {
                project: project.name.clone(),
                service: service.name.clone(),
                image: service.image.clone(),
                registry: registry.clone(),
                // No credential in a derived instruction. The one the far
                // node stored when the copy was placed is still there, and
                // minting a fresh token on every pass would make every
                // payload differ from the last — which is the comparison
                // this relies on to stay quiet.
                username: None,
                secret: None,
                env: service.env.clone(),
                port: ports
                    .iter()
                    .find(|port| port.hostname.is_some())
                    .map(|port| port.container_port),
                slots,
                running,
            }) {
                Ok(payload) => payload,
                Err(error) => {
                    tracing::warn!(%error, "could not build a placement instruction");
                    continue;
                }
            };

            match crate::network::errand::queue_if_changed(
                &self.database,
                &node_id,
                crate::network::errand::Kind::Host,
                &format!("placement:{}", service.id),
                &payload,
            )
            .await
            {
                Ok(Some(_)) => tracing::info!(
                    service = %service.slug,
                    node = %node_id,
                    running,
                    letting_go = holding == Holding::LetGo,
                    "told a node what it holds for this service"
                ),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, node = %node_id, "could not queue a placement")
                }
            }
        }
    }

    /// Where this database's primary can be reached **from another
    /// node**: this node's overlay address, and the port bound to it.
    ///
    /// Never the container's own address. A bridge subnet is identical
    /// on every node, so a container address names a different container
    /// on each machine that reads it — the rule phase 7 already learned
    /// the hard way.
    async fn primary_overlay(&self, service: &Service) -> DeployResult<Option<(String, u16)>> {
        let Some(row) = databases::of_service(&self.database, &service.id).await? else {
            return Ok(None);
        };
        let Some(overlay) = self.overlay_address().await else {
            return Ok(None);
        };
        let primary = replicas::of_service(&self.database, &service.id)
            .await?
            .into_iter()
            .find(|replica| replica.slot == row.primary_slot && replica.is_here());

        Ok(primary
            .and_then(|replica| replica.overlay_port)
            .map(|port| (overlay, port)))
    }

    /// Notice a renewed certificate and hand it to the servers running
    /// on it, without restarting them.
    ///
    /// Convergent, and it asks about the thing rather than about the
    /// history: for every managed copy here, is the certificate on its
    /// volume the one the store holds? A renewal is then just a
    /// difference, whether it came from ACME twelve hours ago or from a
    /// name that changed a minute ago.
    ///
    /// `SIGHUP`, not a redeployment. Postgres re-reads its TLS files on
    /// it, so a certificate that expires every ninety days does not cost
    /// an outage every ninety days.
    pub async fn refresh_certificates(&self) -> DeployResult<usize> {
        let client = Containerd::connect().await?;
        let data_dir = &self.config.node.data_dir;
        let projects = projects::all(&self.database).await?;
        let mut refreshed = 0;

        for service in services::all(&self.database, None).await? {
            if !service.kind.is_managed() {
                continue;
            }
            let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
                continue;
            };
            let names = self.certificate_names(project, &service).await;
            let Some(primary) = names.first() else {
                continue;
            };
            // What the store holds, or a new leaf from the node's CA.
            //
            // Read from **where the certificate came from** rather than
            // from the policy: `source` exists because the renewal loop
            // used to read `issuer` as a decision and silently replaced
            // anything it did not recognise. ACME's and a file's belong
            // to whoever keeps them fresh; ours is ours to reissue, and
            // `ensure_self_signed` is convergent — it returns the stored
            // one untouched while it is fresh and still covers the
            // names.
            //
            // Without this a self-signed database certificate simply
            // expired: nothing else on the node reissues one, because a
            // database's internal name is not a name the edge was asked
            // to serve.
            use crate::edge::certs::Source;
            let stored = match crate::edge::certs::load(&self.database, primary).await {
                Ok(Some(found)) if matches!(found.source, Source::Acme | Source::File) => found,
                _ => match crate::edge::certs::ensure_self_signed(&self.database, primary, &names)
                    .await
                {
                    Ok(issued) => issued,
                    Err(error) => {
                        tracing::warn!(name = %primary, %error, "could not issue a certificate");
                        continue;
                    }
                },
            };

            for replica in replicas::of_service(&self.database, &service.id).await? {
                if !replica.is_here() || replica.evicted() {
                    continue;
                }
                let id = replica.container_id(&project.slug, &service.slug);
                let on_disk =
                    std::fs::read_to_string(database::tls_dir(data_dir, &id).join("server.crt"))
                        .unwrap_or_default();
                if on_disk == stored.cert_pem {
                    continue;
                }

                database::write_tls(data_dir, &id, &stored.cert_pem, &stored.key_pem)?;
                match containers::signal(&client, &id, 1).await {
                    Ok(()) => {
                        tracing::info!(service = %service.slug, slot = replica.slot,
                            "handed a renewed certificate to a running server");
                        refreshed += 1;
                    }
                    // Not running is not a failure: the file is in
                    // place, and the next start reads it.
                    Err(error) => {
                        tracing::debug!(container = %id, %error, "could not signal")
                    }
                }
            }
        }
        Ok(refreshed)
    }

    async fn certificate_names(&self, project: &Project, service: &Service) -> Vec<String> {
        certificate_names(&self.database, &self.config, project, service).await
    }

    /// Put this copy's certificate in its volume, and make sure the
    /// server can read it.
    ///
    /// The certificate comes from the store when the name has one —
    /// which is where ACME puts it, and where `edge::policy` decides
    /// what "renew" means for that name — and is signed by the node's
    /// own CA otherwise. So a database has TLS from its first second,
    /// under whichever source somebody chose, and giving it a public
    /// name later replaces the certificate without changing anything
    /// here.
    ///
    /// Ownership is settled by **asking the image**. Postgres reads the
    /// key as its own unprivileged user, that user is 70 on the alpine
    /// variant and 999 on the debian one, and a node that hard-coded
    /// either would be right until somebody changed the tag. A one-shot
    /// container runs `chown` instead, and only when the file still
    /// belongs to root.
    async fn place_certificate(
        &self,
        client: &Containerd,
        service: &Service,
        names: &[String],
        container_id: &str,
    ) -> DeployResult<()> {
        let Some(primary) = names.first() else {
            return Err(DeployError::Refused(
                "a database needs a name to have a certificate for".into(),
            ));
        };
        let data_dir = &self.config.node.data_dir;

        // What the store holds for this name, or a leaf from the node's
        // CA. `ensure_self_signed` is convergent: it reissues only when
        // the names change or expiry is near.
        let stored = match crate::edge::certs::load(&self.database, primary).await {
            Ok(Some(found)) => found,
            _ => crate::edge::certs::ensure_self_signed(&self.database, primary, names)
                .await
                .map_err(|error| DeployError::Refused(error.to_string()))?,
        };
        database::write_tls(data_dir, container_id, &stored.cert_pem, &stored.key_pem)?;

        // The archive directory has the same fault as the key: made by
        // the node as root, written by a server that is not. Fixed in
        // the same pass so a database needs one container for both
        // rather than one each.
        let archiving = crate::node::settings::archiving(&self.database).await;
        let fix_archive = archiving && database::archive_owner_is_wrong(data_dir, container_id);

        if database::tls_owner_is_wrong(data_dir, container_id) || fix_archive {
            let fixer = format!("{container_id}.chown");
            let mut mounts = vec![BindMount {
                source: volumes::ensure(data_dir, container_id, postgres::VOLUME)?,
                destination: postgres::DATA_MOUNT.to_string(),
                read_only: false,
            }];
            let mut what = format!(
                "chown -R postgres:postgres {dir} && chmod 0600 {key}",
                dir = postgres::TLS_DIR,
                key = postgres::key_path()
            );
            if fix_archive {
                mounts.push(BindMount {
                    source: database::archive_dir(data_dir, container_id),
                    destination: postgres::ARCHIVE_MOUNT.to_string(),
                    read_only: false,
                });
                what.push_str(&format!(
                    " && chown postgres:postgres {}",
                    postgres::ARCHIVE_MOUNT
                ));
            }

            let request = ContainerRequest {
                // `postgres` by name, resolved inside the image by the
                // image. That is the whole point of doing this here.
                command: vec!["sh".into(), "-c".into(), what],
                mounts,
                ..Default::default()
            };
            let credential = crate::platform::registry_credentials::for_reference(
                &self.database,
                &service.image,
            )
            .await;
            let log = logs::prepare(data_dir, &fixer).ok();
            let code = containers::run_to_completion(
                client,
                &fixer,
                &service.image,
                &request,
                credential.as_ref(),
                log.as_deref(),
                std::time::Duration::from_secs(120),
            )
            .await?;
            if code != 0 {
                let said = logs::tail(data_dir, &fixer, 500).unwrap_or_default();
                return Err(DeployError::Refused(format!(
                    "could not give the key to the server's own user (exit {code}): {said}"
                )));
            }
            logs::discard(data_dir, &fixer);
        }
        Ok(())
    }

    /// Copy one running database into a directory, with its own tool.
    ///
    /// **A file copy of a running data directory is not a backup.** It
    /// is a copy taken mid-write, which restores into a server that will
    /// not start — the same class of mistake as copying SQLite by hand.
    /// So the engine takes it: `pg_basebackup`, the same tool and the
    /// same container pattern that seeds a standby, producing something
    /// Postgres opens.
    ///
    /// **From a read-only copy when there is one.** That is the concrete
    /// benefit of having standbys: a base backup is a full read of the
    /// database, and taking it from the primary spends that on the
    /// machine answering the writes. Falls back to the primary when
    /// there is no other copy here, because a backup from the wrong
    /// place beats no backup.
    ///
    /// Returns what it copied, or why it could not. `Ok` means the tool
    /// exited zero and the directory holds what it wrote — nothing here
    /// verifies the archive itself, and a restore is what does.
    pub async fn back_up_engine(
        &self,
        project: &Project,
        service: &Service,
        into: &std::path::Path,
    ) -> Result<u64, String> {
        let row = databases::of_service(&self.database, &service.id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "no engine row".to_string())?;
        let placements = replicas::of_service(&self.database, &service.id)
            .await
            .map_err(|error| error.to_string())?;

        // A standby first, the primary second. Both must be here and
        // running: a copy elsewhere is that node's to back up, and one
        // with no address is not answering.
        let from = placements
            .iter()
            .filter(|replica| replica.is_here() && !replica.evicted())
            .filter(|replica| replica.address.is_some())
            .min_by_key(|replica| (replica.slot == row.primary_slot, replica.slot))
            .ok_or_else(|| "no copy of it is running here".to_string())?;
        let address = from.address.clone().unwrap_or_default();

        std::fs::create_dir_all(into).map_err(|error| error.to_string())?;
        let client = Containerd::connect().await.map_err(|e| e.to_string())?;
        let id = format!("{}.backup", from.container_id(&project.slug, &service.slug));

        let request = ContainerRequest {
            // **The replication role, not the admin one.** A base backup
            // is a replication connection, and `pg_hba.conf` admits
            // those for exactly one user — so the admin's credentials
            // are refused by the line that exists to refuse everybody
            // else. Found on the node, first run: `no pg_hba.conf entry
            // for replication connection from host "10.42.2.1", user
            // "orders"`. The seeder had it right and this copied the
            // shape without the reason.
            command: postgres::base_backup_into(&address, postgres::PORT, &row.replication_user),
            env: vec![("PGPASSWORD".to_string(), row.replication_password.clone())],
            mounts: vec![BindMount {
                source: into.to_path_buf(),
                destination: postgres::BACKUP_MOUNT.to_string(),
                read_only: false,
            }],
            // The host's network, like the seeder: the copy it reads
            // from is on this machine's bridge, and a container in its
            // own namespace would need an address of its own to reach
            // it.
            ..Default::default()
        };

        let data_dir = &self.config.node.data_dir;
        let log = logs::prepare(data_dir, &id).map_err(|error| error.to_string())?;
        let outcome = containers::run_to_completion(
            &client,
            &id,
            &service.image,
            &request,
            None,
            Some(&log),
            BACKUP_DEADLINE,
        )
        .await;
        let said = logs::tail(data_dir, &id, 2_000).unwrap_or_default();
        logs::discard(data_dir, &id);

        match outcome {
            Ok(0) => Ok(crate::node::disk::used(into).bytes),
            Ok(code) => Err(format!("pg_basebackup exited {code}: {said}")),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Ask every primary here what its standbys are doing, and write it
    /// down.
    ///
    /// **Phase 5**, and the failure it makes visible: a standby can be
    /// up, healthy by every measure this node had, and no longer
    /// replicating. The container runs, the process answers, memory and
    /// CPU and disk read normally — and the data is frozen at whatever
    /// moment it stopped. Somebody reading from it gets answers, and
    /// they are old, and nothing said so.
    ///
    /// Asked of the *primary*, because that is the only thing that
    /// knows: `pg_replication_slots` has a row per standby whether or
    /// not it is connected, where `pg_stat_replication` simply omits one
    /// that stopped — absence, which cannot be told from a standby
    /// nobody ever made.
    ///
    /// **Only the primaries on this node.** A database whose primary
    /// lives elsewhere is that node's to ask, and the answer would have
    /// to travel — which is the shape reporting already has and is where
    /// this goes when promotion exists. Today every primary is on the
    /// node that owns the database, because nothing moves one.
    ///
    /// One container per database per pass, ~134 ms measured on the
    /// node. Nothing is recorded when the ask fails: a primary that
    /// could not be reached says nothing about whether a standby is
    /// following, and writing "not following" from a failed question
    /// would be this making the outage up.
    pub async fn ask_replication(&self) -> usize {
        let Ok(services) = services::all(&self.database, None).await else {
            return 0;
        };
        let mut asked = 0;

        for service in services.iter().filter(|service| service.kind.is_managed()) {
            let Ok(Some(row)) = databases::of_service(&self.database, &service.id).await else {
                continue;
            };
            let Ok(Some(project)) = projects::find(&self.database, &service.project_id).await
            else {
                continue;
            };
            let Ok(placements) = replicas::of_service(&self.database, &service.id).await else {
                continue;
            };
            // The primary, and only if it is here and running.
            let Some(primary) = placements
                .iter()
                .find(|replica| replica.slot == row.primary_slot && replica.is_here())
                .filter(|replica| !replica.evicted())
            else {
                continue;
            };
            let Some(address) = &primary.address else {
                continue;
            };

            match self
                .ask_one(&project, service, &row, address, primary.slot)
                .await
            {
                Ok(states) => {
                    for state in states {
                        let Some(replica) =
                            placements.iter().find(|replica| replica.slot == state.slot)
                        else {
                            continue;
                        };
                        if let Err(error) = replicas::set_following(
                            &self.database,
                            &replica.id,
                            state.active,
                            state.held_bytes.max(0) as u64,
                        )
                        .await
                        {
                            tracing::warn!(%error, slot = state.slot, "could not record it");
                        }
                        if !state.active {
                            tracing::warn!(
                                service = %service.slug, slot = state.slot,
                                held = state.held_bytes,
                                "a read-only copy is not following its primary"
                            );
                        }
                    }
                    asked += 1;
                }
                // Debug, not warn: a primary that is starting, or a node
                // between deployments, cannot answer — and this runs on
                // a timer, so a warning would be one a minute for as
                // long as it lasted.
                Err(reason) => {
                    tracing::debug!(service = %service.slug, %reason, "could not ask the primary")
                }
            }
        }
        asked
    }

    /// One question, in a container of the engine's own image.
    async fn ask_one(
        &self,
        project: &Project,
        service: &Service,
        row: &crate::platform::databases::Database,
        address: &str,
        slot: u32,
    ) -> Result<Vec<postgres::SlotState>, String> {
        let client = Containerd::connect().await.map_err(|e| e.to_string())?;
        let data_dir = &self.config.node.data_dir;
        let asker = format!(
            "{}.ask",
            crate::platform::replicas::container_id_for(&project.slug, &service.slug, slot)
        );

        let request = ContainerRequest {
            command: postgres::ask_slots(address, postgres::PORT, &row.admin_user),
            env: vec![("PGPASSWORD".to_string(), row.admin_password.clone())],
            // The host's network — the primary is on this machine's
            // bridge, which the node can reach and a container in its
            // own namespace could not without an address of its own.
            ..Default::default()
        };

        let log = logs::prepare(data_dir, &asker).map_err(|e| e.to_string())?;
        let outcome = containers::run_to_completion(
            &client,
            &asker,
            &service.image,
            &request,
            None,
            Some(&log),
            ASK_DEADLINE,
        )
        .await;

        let said = logs::tail(data_dir, &asker, 4_000).unwrap_or_default();
        logs::discard(data_dir, &asker);

        match outcome {
            Ok(0) => Ok(postgres::parse_replication(&said)),
            Ok(code) => Err(format!("psql exited {code}: {said}")),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Unpack a base backup into a copy that is being restored.
    ///
    /// One container of the engine's own image, which is where `tar`,
    /// `gunzip` and the `postgres` user are — the same reasoning as the
    /// seeder, the health check and the ownership fixer.
    ///
    /// **A failure here must stop the deployment.** The alternative is a
    /// server started on an empty or half-unpacked data directory: the
    /// first runs `initdb` and produces a database that looks restored
    /// and holds nothing, and the second is a cluster with some of its
    /// files. Both start. Both are wrong in a way somebody discovers by
    /// reading rows that are not there.
    async fn unpack_base(
        &self,
        client: &Containerd,
        service: &Service,
        id: &str,
        restoring: &database::Restoring,
    ) -> DeployResult<()> {
        let data_dir = &self.config.node.data_dir;
        let unpacker = format!("{id}.unpack");

        let request = ContainerRequest {
            command: postgres::unpack_base(),
            mounts: vec![
                BindMount {
                    source: volumes::ensure(data_dir, id, postgres::VOLUME)?,
                    destination: postgres::DATA_MOUNT.to_string(),
                    read_only: false,
                },
                // Read-only, because this is somebody else's backup and
                // the copy being made from it must not be able to touch
                // it.
                BindMount {
                    source: restoring.base.clone(),
                    destination: postgres::BASE_MOUNT.to_string(),
                    read_only: true,
                },
            ],
            ..Default::default()
        };

        let log = logs::prepare(data_dir, &unpacker).ok();
        let outcome = containers::run_to_completion(
            client,
            &unpacker,
            &service.image,
            &request,
            None,
            log.as_deref(),
            BACKUP_DEADLINE,
        )
        .await;
        let said = logs::tail(data_dir, &unpacker, 1_000).unwrap_or_default();
        logs::discard(data_dir, &unpacker);

        match outcome {
            Ok(0) => {
                tracing::info!(
                    service = %service.slug, from = %restoring.base.display(),
                    target = restoring.target.as_deref().unwrap_or("the end of the log"),
                    "unpacked a base backup; the server will replay from here"
                );
                Ok(())
            }
            Ok(code) => Err(DeployError::Refused(format!(
                "could not unpack the backup (exit {code}): {said}"
            ))),
            Err(error) => Err(DeployError::Refused(error.to_string())),
        }
    }

    /// Copy the primary's data directory into a standby's empty volume.
    ///
    /// A container of its own, run to completion before the standby is
    /// started: the same image, `pg_basebackup` instead of the server,
    /// the volume mounted where the server will find it.
    ///
    /// ## On the host's network, not the project's
    ///
    /// It reaches the primary at the address it will use for ever after
    /// — a bridge address when the primary is here, an overlay one when
    /// it is elsewhere — and the node can reach both without a namespace
    /// of its own. Attaching it to the project's bridge would mean a CNI
    /// address allocated and released for a container that lives for a
    /// minute.
    ///
    /// ## Retried, and then given up on with a reason
    ///
    /// The primary is running before a standby is asked for, but running
    /// is not accepting connections: `initdb` takes seconds and the
    /// first attempt usually lands inside them. So it tries again for a
    /// bounded while and then stops — a failure is an answer, and this
    /// node's rule is that retrying past that is something somebody
    /// asks for.
    async fn seed_standby(
        &self,
        client: &Containerd,
        service: &Service,
        row: &crate::platform::databases::Database,
        container_id: &str,
        endpoint: (String, u16),
        slot: u32,
    ) -> DeployResult<()> {
        let data_dir = &self.config.node.data_dir;
        if database::seeded(data_dir, container_id) {
            return Ok(());
        }

        let source = volumes::ensure(data_dir, container_id, postgres::VOLUME)?;
        let seeder = format!("{container_id}.seed");
        let request = ContainerRequest {
            command: postgres::base_backup(
                &endpoint.0,
                endpoint.1,
                &row.replication_user,
                &postgres::slot_name(slot),
            ),
            env: vec![
                // In the environment, not the command: a command is in
                // the container's spec on disk and in `ctr containers
                // info`.
                ("PGPASSWORD".to_string(), row.replication_password.clone()),
            ],
            mounts: vec![BindMount {
                source,
                destination: postgres::DATA_MOUNT.to_string(),
                read_only: false,
            }],
            // The host's network — see above — and so no `resolv.conf`
            // of the node's either: it already has the host's.
            ..Default::default()
        };

        let credential =
            crate::platform::registry_credentials::for_reference(&self.database, &service.image)
                .await;

        let mut last = String::new();
        for attempt in 1..=SEED_ATTEMPTS {
            let log = logs::prepare(data_dir, &seeder).ok();
            let outcome = containers::run_to_completion(
                client,
                &seeder,
                &service.image,
                &request,
                credential.as_ref(),
                log.as_deref(),
                SEED_DEADLINE,
            )
            .await;

            match outcome {
                Ok(0) => {
                    tracing::info!(service = %service.slug, slot, attempt, "seeded a standby");
                    logs::discard(data_dir, &seeder);
                    return Ok(());
                }
                // What the tool said, which is the whole reason the log
                // exists: an exit code alone sends somebody to read a
                // container that has already been removed.
                Ok(code) => {
                    last = logs::tail(data_dir, &seeder, 1_000)
                        .unwrap_or_else(|| format!("pg_basebackup exited {code}"));
                }
                Err(error) => last = error.to_string(),
            }

            tracing::warn!(
                service = %service.slug, slot, attempt, reason = %last,
                "could not seed a standby yet"
            );
            if attempt < SEED_ATTEMPTS {
                tokio::time::sleep(SEED_PAUSE).await;
            }
        }

        Err(DeployError::Refused(format!(
            "could not copy the primary into this replica after {SEED_ATTEMPTS} attempts: {last}"
        )))
    }

    /// Throw away everything this service's copies here stored.
    ///
    /// **Irreversible, and the only caller is a deletion somebody
    /// confirmed.** Nothing else on the node removes a volume: a
    /// directory whose rows went away is an orphan `doctor` reports and
    /// somebody can still recover from, and that is the right side to
    /// err on for the one copy of a database.
    ///
    /// Before the rows, always. The directory is named after the
    /// container id, which is derived from the rows — so deleting them
    /// first leaves bytes on the disk that nothing can name.
    pub async fn discard_storage(&self, project: &Project, service: &Service) {
        let placements = match replicas::of_service(&self.database, &service.id).await {
            Ok(placements) => placements,
            Err(error) => {
                tracing::warn!(service = %service.slug, %error, "could not read what to discard");
                return;
            }
        };

        for replica in placements.iter().filter(|replica| replica.is_here()) {
            let id = replica.container_id(&project.slug, &service.slug);
            if let Err(error) = volumes::discard(&self.config.node.data_dir, &id) {
                // Said out loud and carried on from: the rows are going
                // either way, and storage this node could not remove is
                // something `doctor` will list rather than something
                // that should stop a deletion somebody asked for.
                tracing::warn!(container = %id, %error, "could not discard a copy's data");
            }
            database::discard(&self.config.node.data_dir, &id);
            // The names it was given and what it said. Neither is data
            // anybody wants back — both are rebuilt from the rows — but
            // a file per container that ever existed is a directory
            // nobody prunes.
            hosts::discard(&self.config.node.data_dir, &id);
            logs::discard(&self.config.node.data_dir, &id);
        }
    }

    /// What a managed engine needs, or `None` for an ordinary
    /// container.
    ///
    /// Reads the rows and writes the generated files. The arithmetic is
    /// `platform::postgres`, which is pure; this is the part that has
    /// to know where the other copies are.
    async fn prepare_engine(
        &self,
        project: &Project,
        service: &Service,
        replica: &Replica,
        net: &ProjectNetwork,
    ) -> DeployResult<Option<database::Preparation>> {
        if !service.kind.is_managed() {
            return Ok(None);
        }
        let Some(row) = databases::of_service(&self.database, &service.id).await? else {
            // A managed service with no engine row is a service this
            // node cannot start correctly, and starting it as a plain
            // container would be an empty database that looks like it
            // worked.
            return Err(DeployError::Refused(format!(
                "{} is a managed database with no engine row",
                service.slug
            )));
        };

        // An address of its own, for any copy that has not got one yet.
        //
        // Here rather than only in `databases::create`, because a copy
        // added afterwards — from the placement form, or by an errand —
        // goes through neither. On a node, the second replica of a
        // database came up on whatever `host-local` had next and with no
        // reservation at all, so its address was one nothing would keep.
        if replica.is_here() && replica.reserved_host.is_none() {
            replicas::reserve_host(&self.database, &project.id, &replica.id).await?;
        }
        // Read after reserving, so `wanted` below sees it on the first
        // deployment rather than the second.
        let placements = replicas::of_service(&self.database, &service.id).await?;
        let nodes = crate::network::all(&self.database).await?;
        let role = row.role_of(replica.slot);

        // Where a standby dials. Refused rather than started without
        // one: a standby with nowhere to follow would come up holding a
        // copy of somebody's data and answering as though it were
        // current.
        let primary = match role {
            // A restored copy dials nobody: it replays an archive
            // rather than following a server.
            postgres::Role::Primary | postgres::Role::Restoring => None,
            postgres::Role::Standby => {
                let endpoint = self.primary_endpoint(&row, &placements, &nodes, net)?;
                if endpoint.is_none() {
                    return Err(DeployError::Refused(format!(
                        "the primary of {} has no address this node can reach yet",
                        service.slug
                    )));
                }
                endpoint
            }
        };

        // Everywhere a standby of this database may dial in from.
        //
        // The remote ones are their nodes' overlay addresses. A copy
        // *here* is the one that caught this out: its data directory is
        // seeded by a `pg_basebackup` running in the host's network
        // namespace, so the primary sees the **bridge gateway** — and
        // the subnet line above does not help, because `all` in the
        // database column does not match a replication connection. On a
        // node that was six identical refusals in a row, each naming
        // `10.42.2.1`.
        let standbys = || {
            placements
                .iter()
                .filter(|other| row.role_of(other.slot) == postgres::Role::Standby)
                .filter(|other| !other.evicted())
        };
        let mut replication_from: Vec<String> = standbys()
            .filter_map(|other| other.node_id.as_deref())
            .filter_map(|node_id| {
                nodes
                    .iter()
                    .find(|node| node.id == node_id)
                    .and_then(|node| node.overlay_ip.clone())
                    .map(|address| format!("{address}/32"))
            })
            .collect();
        if standbys().any(|other| other.is_here()) {
            replication_from.push(net.subnet());
        }
        replication_from.sort();
        replication_from.dedup();

        let published = ports::of_service(&self.database, &service.id)
            .await?
            .iter()
            .any(|port| port.host_port.is_some());

        // Said out loud, because the two are the same image and the
        // same page and they are not the same container: one accepts
        // writes and the other refuses them. On a node run this line is
        // how you tell which one just started.
        tracing::info!(
            service = %service.slug,
            slot = replica.slot,
            role = role.as_str(),
            standbys = replication_from.len(),
            "preparing a database copy"
        );

        let archiving = crate::node::settings::archiving(&self.database).await;
        // Where a restore reads from. The archive is the **original's** —
        // a restore replays somebody else's log — and this is the one
        // place the two are connected.
        //
        // The container id comes off the end of the backup path, which
        // is a derivation and worth naming as one: `back_up_engines`
        // writes each volume as `volumes/<container id>`, and
        // `archive_dir` is keyed on the same id. One convention, used at
        // both ends. What guards it is the manifest's `format` — a
        // layout that changed without changing that number would break
        // this quietly, which is what the number is for.
        let restoring = match (&row.restored_from, role) {
            (Some(from), postgres::Role::Restoring) => {
                let base = std::path::PathBuf::from(from);
                let container = base
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default();
                Some(database::Restoring {
                    archive: database::archive_dir(&self.config.node.data_dir, &container),
                    base,
                    target: row.recovery_target.clone(),
                })
            }
            _ => None,
        };

        Ok(Some(database::prepare(&database::Plan {
            archiving,
            restoring,
            data_dir: &self.config.node.data_dir,
            container_id: &replica.container_id(&project.slug, &service.slug),
            database: &row,
            role,
            memory_limit: service.memory_limit,
            subnet: net.subnet(),
            replication_from,
            published,
            primary,
        })?))
    }

    /// Where this database's primary answers, from here.
    ///
    /// Two cases, and they are different addresses rather than two ways
    /// of writing one:
    ///
    /// * **The primary is on this node.** Its own address on the
    ///   project's bridge, which is reserved and so does not move.
    /// * **The primary is elsewhere.** That node's overlay address and
    ///   the port *it* bound for this copy. Never the container's own:
    ///   a bridge address names a different container on every machine,
    ///   which is the rule phase 7 already records.
    ///
    /// `None` when the second case has not settled yet: the port comes
    /// out of the other node's port space and travels home on a report,
    /// so there is a window where the answer is honestly not known.
    fn primary_endpoint(
        &self,
        row: &crate::platform::databases::Database,
        placements: &[Replica],
        nodes: &[crate::network::Node],
        net: &ProjectNetwork,
    ) -> DeployResult<Option<(String, u16)>> {
        // What this node was told, when it was told. A node holding a
        // standby has no row for the primary and never will — the
        // errand carried the address, and reading rows here would find
        // nothing and refuse a deployment that is perfectly possible.
        if let Some(told) = &row.primary_endpoint {
            let (host, port) = told
                .rsplit_once(':')
                .ok_or_else(|| DeployError::Refused(format!("{told:?} is not host:port")))?;
            let port: u16 = port
                .parse()
                .map_err(|_| DeployError::Refused(format!("{told:?} is not host:port")))?;
            return Ok(Some((host.to_string(), port)));
        }

        let Some(primary) = placements
            .iter()
            .find(|replica| replica.slot == row.primary_slot)
        else {
            return Ok(None);
        };

        match &primary.node_id {
            None => match primary.reserved_host {
                Some(host) => Ok(Some((
                    net.reserved_address(host)?.to_string(),
                    postgres::PORT,
                ))),
                None => Ok(None),
            },
            Some(node_id) => {
                let overlay = nodes
                    .iter()
                    .find(|node| &node.id == node_id)
                    .and_then(|node| node.overlay_ip.clone());
                Ok(match (overlay, primary.overlay_port) {
                    (Some(address), Some(port)) => Some((address, port)),
                    _ => None,
                })
            }
        }
    }

    /// The directories this copy's volumes live in, made if they are
    /// not there.
    ///
    /// Keyed on the container id, so two copies on one node get two
    /// directories — the rule `platform::volumes` exists to hold, and
    /// the one that stops two database servers writing one data
    /// directory.
    fn mounts_for(&self, container_id: &str, declared: &[Volume]) -> DeployResult<Vec<BindMount>> {
        declared
            .iter()
            .map(|volume| {
                let source =
                    volumes::ensure(&self.config.node.data_dir, container_id, &volume.name)?;
                Ok(BindMount {
                    source,
                    destination: volume.path.clone(),
                    read_only: false,
                })
            })
            .collect()
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

/// The names a database's certificate has to cover.
///
/// The short ones a container in the project would use, and the
/// qualified one that is also the public name — the only one a public
/// authority can sign, and so the only one that verifies when the policy
/// for it is `Acme`.
///
/// A free function because the certificate loop needs the same answer
/// and has no `Deployer`: a database's names are not names the edge was
/// asked to serve, so anything reasoning about what this node still
/// wants a certificate for has to ask here too.
pub async fn certificate_names(
    database: &SqliteDatabase,
    config: &crate::config::Config,
    project: &Project,
    service: &Service,
) -> Vec<String> {
    // The **owner's** domain when this copy is held for somebody else. A
    // certificate for a name built from this machine's domain would be a
    // name no client writes: the database is not this node's, and
    // neither is what it is called.
    let domain = match databases::of_service(database, &service.id).await {
        Ok(Some(row)) if row.owner_domain.is_some() => row.owner_domain,
        _ => crate::node::settings::domain(database, config).await,
    };
    // The name the operator chose, or the one every database had before
    // they could — one function, so the certificate covers exactly what
    // `/etc/hosts` resolves. Two builders of this drifted once already and
    // the failure was a client dialling a name no certificate held.
    let qualified = databases::qualified_name(database, service, &project.slug, domain.as_deref())
        .await
        .ok()
        .flatten();
    let mut names = Vec::new();

    // Both names, and the read pool's is not optional: a client
    // connecting to `orders-ro` with `verify-full` checks the name it
    // dialled against the certificate, and a certificate that covered
    // only the primary's name would fail every read. The pool is the
    // same database, so it is the same certificate with one more name on
    // it rather than a second certificate.
    for (slug, qualified) in [
        (service.slug.clone(), qualified.clone()),
        (
            format!("{}{}", service.slug, hosts::READ_ONLY),
            qualified.as_deref().map(hosts::pool_name),
        ),
    ] {
        if let Some(qualified) = qualified {
            // First, so the qualified one is the common name: it is the
            // only one a public authority could sign, and the one a
            // client outside this node would use.
            names.push(qualified);
        }
        names.push(slug.clone());
        names.push(format!("{slug}.{}", project.slug));
    }
    names
}

/// Every name this node stores a **database** certificate under.
///
/// The first name of each managed service's set, because that is the key
/// `refresh_certificates` writes it under — see there.
/// The cgroup a pid is in, on a v2 tree.
///
/// One line, `0::/the/path`. A v1 tree has one line per controller and no
/// unified path, which is a node this product refuses at install — see
/// `preflight`.
fn cgroup_of(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("0::").map(str::to_string))
}

/// The names a **public** authority should be asked to sign for one key.
///
/// Every other name is itself. A database is two: the qualified primary
/// and the qualified read pool, which change together and have to be on
/// one certificate — a client dialling `orders-ro` with `verify-full`
/// checks the name it asked for, so an order carrying only the primary is
/// every read failing.
///
/// **And only the qualified ones.** `orders` and `orders.db-test` resolve
/// inside the project and nowhere else, so no authority can validate them:
/// there is no challenge to pass for a name that does not exist outside
/// this node. Choosing a public authority for a database therefore costs
/// the short names their verification — the page says so, and this is
/// where it becomes true.
pub async fn public_names_for(
    database: &SqliteDatabase,
    config: &crate::config::Config,
    key: &str,
) -> Vec<String> {
    let projects = projects::all(database).await.unwrap_or_default();
    for service in services::all(database, None).await.unwrap_or_default() {
        if !service.kind.is_managed() {
            continue;
        }
        let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
            continue;
        };
        let names = certificate_names(database, config, project, &service).await;
        if names.first().map(String::as_str) != Some(key) {
            continue;
        }
        // Qualified only, which is what having a dot past the project
        // means here — see `certificate_names`, which puts them first.
        return vec![key.to_string(), hosts::pool_name(key)];
    }
    vec![key.to_string()]
}

pub async fn database_certificate_keys(
    database: &SqliteDatabase,
    config: &crate::config::Config,
) -> DeployResult<Vec<String>> {
    let projects = projects::all(database).await?;
    let mut keys = Vec::new();
    for service in services::all(database, None).await? {
        if !service.kind.is_managed() {
            continue;
        }
        let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
            continue;
        };
        if let Some(primary) = certificate_names(database, config, project, &service)
            .await
            .into_iter()
            .next()
        {
            keys.push(primary);
        }
    }
    Ok(keys)
}

/// How long a container gets to exit before it is killed.
const STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// How many times a standby's seed is tried, and how long it waits
/// between.
///
/// The primary is up before a standby is asked for, but running is not
/// accepting connections — `initdb` takes seconds, and the first attempt
/// lands inside them. Six tries over a minute covers that without
/// holding a job for the five minutes a real outage would take.
/// How long a container gets to fail at starting before the deploy path
/// stops watching. Long enough for Postgres to read its configuration
/// and refuse it, short enough not to hold the job.
const SETTLE: std::time::Duration = std::time::Duration::from_secs(3);

const SEED_ATTEMPTS: u32 = 6;
const SEED_PAUSE: std::time::Duration = std::time::Duration::from_secs(10);

/// How long one `pg_basebackup` gets. Generous, because it is copying a
/// database over a network — and bounded, because a job that never ends
/// is a queue that never moves.
const SEED_DEADLINE: std::time::Duration = std::time::Duration::from_secs(1800);

/// How long one question to a primary may take.
///
/// Short: it is a connect and a single-row query against a server on
/// this machine's own bridge, measured at 134 ms end to end including
/// the container. A generous deadline here would mean a pass that takes
/// longer than the interval between passes when a primary is unwell —
/// which is exactly when the pass matters.
/// How long a base backup may take.
///
/// Half an hour, which is the seeding deadline's reasoning at a
/// different scale: this reads the whole database over a socket, and a
/// deadline shorter than the data is a backup that can never succeed on
/// a large one. It is a bound against a hang, not a promise about speed.
const BACKUP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(1800);

const ASK_DEADLINE: std::time::Duration = std::time::Duration::from_secs(15);

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

    /// A service, a node that has agreed to run containers for us, and a
    /// copy placed there.
    async fn placed_elsewhere() -> (SqliteDatabase, Project, Service) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "shared")
            .await
            .expect("project");
        let service = services::create(
            &database,
            &project.id,
            "web",
            "registry.example/web@sha256:abc",
            &[],
        )
        .await
        .expect("service");

        crate::network::save(
            &database,
            &crate::network::Node {
                id: "nd-far".into(),
                name: "far.example".into(),
                kind: crate::network::Kind::Private,
                overlay_ip: Some("10.42.0.9".into()),
                public_key: None,
                endpoint: None,
                allows: vec![crate::network::capability::Capability::Host],
                ca_pem: None,
                last_seen_at: None,
                is_self: false,
            },
        )
        .await
        .expect("node");

        // Creating a service already places its first copy here, so this
        // moves that one rather than adding a second in the same slot.
        let here = replicas::of_service(&database, &service.id)
            .await
            .expect("replicas")
            .pop()
            .expect("a service is created with one copy");
        replicas::move_to(&database, &here.id, Some("nd-far"))
            .await
            .expect("moved");
        (database, project, service)
    }

    /// **A stop has to travel.** Stopping a service took down the copies
    /// on this node and said nothing to the machines running the others,
    /// so a service the console showed as stopped went on serving traffic
    /// somewhere else. Found by Jorge on the test nodes.
    #[tokio::test]
    async fn stopping_a_service_tells_the_nodes_holding_its_other_copies() {
        let (database, project, service) = placed_elsewhere().await;
        let deployer = Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::config::Config::default(),
        );

        // Through `stop`, not through the dispatcher. The first version of
        // this set the state, re-read the row and handed the fresh struct
        // over — which is the arrangement the code should have made for
        // itself, so it passed while a real stop travelled as
        // `running: true`. Containerd is not here, so the local half
        // fails; the instruction leaves before it.
        let _ = deployer.stop(&project, &service).await;
        assert_eq!(
            services::find(&database, &service.id)
                .await
                .expect("query")
                .expect("there")
                .desired_state,
            DesiredState::Stopped,
            "the intent is written even when the runtime will not answer"
        );

        let waiting = crate::network::errand::waiting(&database, "nd-far")
            .await
            .expect("errands");
        assert_eq!(waiting.len(), 1, "the far node was told nothing");
        assert_eq!(waiting[0].payload["running"], serde_json::json!(false));
        assert_eq!(
            waiting[0].payload["slots"],
            serde_json::json!([1]),
            "stopped, and still placed there — `slots: []` would delete it"
        );
    }

    /// Deleting a service has to reach the machines running its copies,
    /// and it is the one instruction with no second chance.
    ///
    /// Everything else in this file is recomputed at the next boot from
    /// rows that are still here. After a deletion there are none, so a
    /// copy this never reached would go on running somewhere else with
    /// nobody left to contradict it — which is why the withdrawal leaves
    /// before the rows do, and why it is `slots: []` rather than a stop:
    /// "stop and keep it" is the opposite of what a deletion means.
    #[tokio::test]
    async fn deleting_a_service_tells_the_holders_to_let_it_go() {
        let (database, project, service) = placed_elsewhere().await;
        let deployer = Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::config::Config::default(),
        );

        // Through `tear_down`, which is what the delete handler calls, and
        // whose local half needs a containerd that is not here.
        deployer.tear_down(&project, &service).await;

        let waiting = crate::network::errand::waiting(&database, "nd-far")
            .await
            .expect("errands");
        let told = waiting.last().expect("the far node was told nothing");
        assert_eq!(
            told.payload["slots"],
            serde_json::json!([]),
            "a deletion is not a stop: nothing is left to hold"
        );
        assert_eq!(told.payload["running"], serde_json::json!(false));
    }

    /// A managed database owned here, with a standby on a node that has
    /// agreed to keep data, and this node on the overlay so the errand can
    /// say where the primary answers.
    async fn database_with_a_standby_elsewhere() -> (SqliteDatabase, Project, Service) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "db-test")
            .await
            .expect("project");
        let (service, _) = crate::platform::databases::create(
            &database,
            &project.id,
            "orders",
            "17",
            256 * 1024 * 1024,
        )
        .await
        .expect("created");

        // This node, on the overlay: the errand carries where the primary
        // answers, and that is this node's address and a port bound to it.
        crate::network::ensure_self(&database, &crate::config::Config::default())
            .await
            .expect("self");
        let mut me = crate::network::me(&database)
            .await
            .expect("query")
            .expect("there");
        me.overlay_ip = Some("10.42.0.1".into());
        crate::network::save(&database, &me).await.expect("saved");

        let here = replicas::of_service(&database, &service.id)
            .await
            .expect("replicas")
            .pop()
            .expect("a database is created with its primary");
        replicas::ensure_overlay_port(&database, &here.id)
            .await
            .expect("port");

        // And a node that agreed to keep data, holding the standby.
        crate::network::save(
            &database,
            &crate::network::Node {
                id: "nd-far".into(),
                name: "far.example".into(),
                kind: crate::network::Kind::Private,
                overlay_ip: Some("10.42.0.9".into()),
                public_key: None,
                endpoint: None,
                allows: vec![crate::network::capability::Capability::Store],
                ca_pem: None,
                last_seen_at: None,
                is_self: false,
            },
        )
        .await
        .expect("node");
        replicas::place(&database, &service.id, Some("nd-far"), 3)
            .await
            .expect("placed");
        (database, project, service)
    }

    /// A database is told by the other dispatcher, and stopping one has to
    /// reach it.
    ///
    /// This is the test that was missing. The `running` field, the payload
    /// that carries it and the far node's handling of it all shipped
    /// together — and nothing called `dispatch_standbys` on the way out of
    /// `stop`, so Jorge stopped a database from the console and the standby
    /// on the other machine went on following it. `tell_holders` declines a
    /// managed kind at its first line, which is correct and was the whole
    /// of the coverage.
    ///
    /// It runs `stop` rather than the dispatcher directly: what was wrong
    /// was the wiring, and a test of the dispatcher would have passed
    /// before the fix. Containerd is not here, so the local half fails —
    /// which is why the dispatch happens before it, and asserting on the
    /// errand is asserting exactly that.
    #[tokio::test]
    async fn stopping_a_database_tells_the_node_holding_its_standby() {
        let (database, project, service) = database_with_a_standby_elsewhere().await;
        let deployer = Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::config::Config::default(),
        );
        // The local half needs containerd and there is none here. What is
        // being pinned is what left this node before that.
        let _ = deployer.stop(&project, &service).await;

        let waiting = crate::network::errand::waiting(&database, "nd-far")
            .await
            .expect("errands");
        let stop = waiting
            .iter()
            .find(|errand| errand.kind == crate::network::errand::Kind::Database)
            .expect("the node holding the standby was told nothing");
        assert_eq!(stop.payload["running"], serde_json::json!(false));
        assert_eq!(
            stop.payload["slots"],
            serde_json::json!([3]),
            "stopped, and still held — the volume stays where it is"
        );
    }

    /// And deleting one says the other thing: nothing left to hold, which
    /// is what lets the far node take the copy off its own disk. A stop
    /// would leave it there for ever, following a primary that no longer
    /// exists.
    #[tokio::test]
    async fn deleting_a_database_tells_the_holder_to_let_the_copy_go() {
        let (database, project, service) = database_with_a_standby_elsewhere().await;
        let deployer = Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::config::Config::default(),
        );

        deployer.tear_down(&project, &service).await;

        let told = crate::network::errand::waiting(&database, "nd-far")
            .await
            .expect("errands")
            .into_iter()
            .rfind(|errand| errand.kind == crate::network::errand::Kind::Database)
            .expect("the node holding the standby was told nothing");
        assert_eq!(told.payload["slots"], serde_json::json!([]));
        assert_eq!(told.payload["running"], serde_json::json!(false));
    }

    /// **A ceiling is a reservation**, so a node counts what it has
    /// promised and refuses to promise more.
    ///
    /// Three claims, and the middle one is the trap. The count must
    /// exclude what *this* service already holds: counting it means a
    /// ceiling refused by its own current value — a form somebody can
    /// set once and never change their mind in, which is worse than no
    /// check because it looks like a rule.
    ///
    /// And per copy, because a service with three replicas here costs
    /// three times its ceiling. That is what it actually costs.
    #[test]
    fn a_node_refuses_to_promise_more_than_it_has() {
        let gb = 1024 * 1024 * 1024;
        let say = |bytes| crate::node::memory::human(bytes);
        let empty = Room {
            allocatable: 4 * gb,
            known: true,
            committed: 0,
            already: 0,
        };

        assert_eq!(fits(empty, gb, 1, say), None, "nothing promised yet");
        assert!(
            fits(empty, 5 * gb, 1, say).is_some(),
            "more than the machine has"
        );

        // Three copies of 2 GB is 6, which does not fit in 4.
        assert!(fits(empty, 2 * gb, 3, say).is_some());
        assert_eq!(fits(empty, gb, 3, say), None, "and three of 1 GB does");

        // Holding 1 GB and asking for the same again: its own ceiling
        // must not count against it.
        let holding = Room {
            committed: gb,
            already: gb,
            ..empty
        };
        assert_eq!(fits(holding, gb, 1, say), None);
        assert_eq!(fits(holding, 4 * gb, 1, say), None, "and it may grow");
        assert!(fits(holding, 5 * gb, 1, say).is_some(), "up to the machine");

        // Somebody else's 3 GB is not this service's to spend.
        let crowded = Room {
            committed: 3 * gb + gb,
            already: gb,
            ..empty
        };
        assert_eq!(fits(crowded, gb, 1, say), None);
        let refusal = fits(crowded, 2 * gb, 1, say).expect("refused");
        // With the numbers in it: "refused" alone is a form somebody
        // fights with by trying smaller values.
        assert!(refusal.contains("4.0 GB to promise"), "{refusal}");
        assert!(refusal.contains("3.0 GB is already promised"), "{refusal}");
    }

    /// A node that cannot measure itself must not stop somebody setting
    /// a limit.
    ///
    /// The first version had no such case and refused *everything* on
    /// any machine whose total it could not read — found by running the
    /// test above on a laptop with no `/proc/meminfo`, where
    /// `allocatable` came out nought and every ceiling was too big for
    /// it. A reserve you cannot measure is not one you can hold back,
    /// and a rule made of a missing number is not a rule.
    #[test]
    fn what_cannot_be_measured_is_not_enforced() {
        let unknown = Room {
            allocatable: 0,
            known: false,
            committed: 0,
            already: 0,
        };
        assert_eq!(
            fits(
                unknown,
                1024 * 1024 * 1024 * 1024,
                99,
                crate::node::memory::human
            ),
            None,
            "an unreadable machine does not get to refuse"
        );

        // Nought that *was* read is a different answer: a machine with
        // nothing left to promise refuses, and says so.
        let full = Room {
            known: true,
            ..unknown
        };
        assert!(fits(full, 1, 1, crate::node::memory::human).is_some());
    }

    /// A slot that comes back has to come back empty.
    ///
    /// Taking the copies to one and back to two is what somebody does
    /// with a standby that will not start, and it did nothing: the row
    /// went, the directory stayed, the new row got the same container id
    /// and adopted it — so the second standby was the first one's
    /// failure, exactly. A standby's directory holds a copy of the
    /// primary and nothing of its own, which is what makes deleting it
    /// safe and adopting it wrong.
    #[tokio::test]
    async fn a_standbys_data_goes_when_its_row_does_and_the_primarys_stays() {
        let (database, project, service) = database_with_a_standby_elsewhere().await;
        let node = tempfile::tempdir().expect("tempdir");
        let mut config = crate::config::Config::default();
        config.node.data_dir = node.path().to_path_buf();
        let deployer = Deployer::new(std::sync::Arc::new(database.clone()), &config);

        replicas::ensure_here(&database, &service.id, 2)
            .await
            .expect("a copy here");
        let replica = replicas::in_slot(&database, &service.id, 2)
            .await
            .expect("query")
            .expect("there");
        let (primary, standby) = (
            format!("{}.{}", project.slug, service.slug),
            replica.container_id(&project.slug, &service.slug),
        );
        for id in [&primary, &standby] {
            volumes::ensure(node.path(), id, crate::platform::postgres::VOLUME).expect("volume");
        }

        // Containerd is not here, which is the ordinary case in a test
        // and a real one on a node whose runtime is unwell: the row and
        // the directory go either way.
        deployer
            .forget_replica(&project, &service, &replica)
            .await
            .expect("forgotten");

        assert!(
            !volumes::directory(node.path(), &standby, crate::platform::postgres::VOLUME).exists(),
            "the standby's data is gone"
        );
        assert!(
            volumes::directory(node.path(), &primary, crate::platform::postgres::VOLUME).exists(),
            "and the primary's is not — that one is the database"
        );
        assert!(
            replicas::in_slot(&database, &service.id, 2)
                .await
                .expect("query")
                .is_none(),
            "and the row went with it"
        );
    }

    /// Queued only when it differs, because this runs at every boot: the
    /// shape `dispatch_standbys` already uses, and a pass that queued
    /// every time would be a new errand every fifteen seconds.
    #[tokio::test]
    async fn saying_the_same_thing_again_queues_nothing() {
        let (database, project, service) = placed_elsewhere().await;
        let deployer = Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::config::Config::default(),
        );

        deployer
            .tell_holders(&project, &service, Holding::AsPlaced)
            .await;
        deployer
            .tell_holders(&project, &service, Holding::AsPlaced)
            .await;

        assert_eq!(
            crate::network::errand::waiting(&database, "nd-far")
                .await
                .expect("errands")
                .len(),
            1
        );
    }

    /// A node that has taken `host` away is not sent the instruction —
    /// stopping needs the same permission as placing, so it would refuse
    /// it. What that node does instead is throw the copy off itself, at
    /// its own boot, which needs nobody's permission.
    #[tokio::test]
    async fn a_node_that_no_longer_runs_our_containers_is_not_told() {
        let (database, project, service) = placed_elsewhere().await;
        let deployer = Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::config::Config::default(),
        );

        let mut far = crate::network::find(&database, "nd-far")
            .await
            .expect("query")
            .expect("there");
        far.allows = Vec::new();
        crate::network::save(&database, &far).await.expect("saved");

        deployer
            .tell_holders(&project, &service, Holding::AsPlaced)
            .await;
        assert!(crate::network::errand::waiting(&database, "nd-far")
            .await
            .expect("errands")
            .is_empty());
    }

    /// The other half of that rule: what this node no longer agrees to
    /// run, it throws off — because the owner can no longer be the one to
    /// stop it, and a copy nobody can stop would run for ever.
    #[tokio::test]
    async fn a_copy_this_node_no_longer_agrees_to_run_is_thrown_off() {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "theirs")
            .await
            .expect("project");
        let service = services::create(
            &database,
            &project.id,
            "web",
            "registry.example/web@sha256:abc",
            &[],
        )
        .await
        .expect("service");
        services::set_origin(&database, &service.id, "nd-authority")
            .await
            .expect("origin");
        let replica = replicas::of_service(&database, &service.id)
            .await
            .expect("replicas")
            .pop()
            .expect("a service is created with one copy");

        let deployer = Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::config::Config::default(),
        );
        // No grant to that authority at all, which is what revoking one
        // leaves behind.
        deployer.evict_ungranted().await;

        let after = replicas::find(&database, &replica.id)
            .await
            .expect("query")
            .expect("the row is the tombstone");
        assert!(after.evicted(), "it is still claimed as something we run");
        assert_eq!(
            after.last_error.as_deref(),
            Some("this node no longer agrees to run it"),
            "and the row says why"
        );
    }

    /// And it asks about now, not about history: a copy whose authority
    /// still has the grant is left exactly alone.
    #[tokio::test]
    async fn a_copy_this_node_still_agrees_to_run_is_left_alone() {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "theirs")
            .await
            .expect("project");
        let service = services::create(
            &database,
            &project.id,
            "web",
            "registry.example/web@sha256:abc",
            &[],
        )
        .await
        .expect("service");
        services::set_origin(&database, &service.id, "nd-authority")
            .await
            .expect("origin");
        let replica = replicas::of_service(&database, &service.id)
            .await
            .expect("replicas")
            .pop()
            .expect("a service is created with one copy");
        crate::network::capability::grant(
            &database,
            "nd-authority",
            &[crate::network::capability::Capability::Host],
        )
        .await
        .expect("grant");

        let deployer = Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::config::Config::default(),
        );
        deployer.evict_ungranted().await;

        assert!(!replicas::find(&database, &replica.id)
            .await
            .expect("query")
            .expect("there")
            .evicted());
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
