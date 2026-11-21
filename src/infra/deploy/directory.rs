//! What a project runs on which node, for the containers that need a copy
//! somewhere else: the owner builds it from its rows and tells every node
//! holding a copy, and each node stands in for what it has no copy of.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};

use super::{DeployResult, Deployer, ProjectNetwork};
use crate::domain::entities::project::Project;
use crate::domain::entities::service::DesiredState;
use crate::infra::network::errand::{self, Copy, Directory, Kind, Listed};
use crate::infra::reach::names::Route;
use crate::infra::reach::{self, Knock, Role};
use crate::infra::repositories::{directories, ports, projects, replicas, services, stand_ins};

/// The slot a stand-in's address goes under, for `hosts::entries_for`: a
/// database's primary slot keeps its name, and any other slot is its pool.
const POOL_SLOT: u32 = u32::MAX;

/// A project's directory, and the nodes it is for.
struct Spread {
    directory: Directory,
    recipients: BTreeSet<String>,
}

impl Deployer {
    pub(crate) async fn overlay_ip(&self) -> Option<Ipv4Addr> {
        self.overlay_address().await?.parse().ok()
    }

    /// The directory this node goes by for `project`: built here when the
    /// project is ours, and as its owner last said otherwise. `None` when
    /// the project runs on this node alone.
    pub(crate) async fn directory(&self, project: &Project) -> DeployResult<Option<Directory>> {
        if !project.is_ours() {
            return Ok(directories::of_project(&self.database, &project.id).await?);
        }
        Ok(self.spread(project).await?.map(|spread| spread.directory))
    }

    async fn spread(&self, project: &Project) -> DeployResult<Option<Spread>> {
        let Some(me) = self.overlay_address().await else {
            return Ok(None);
        };
        let nodes: BTreeMap<String, crate::infra::network::Node> =
            crate::infra::network::all(&self.database)
                .await?
                .into_iter()
                .map(|node| (node.id.clone(), node))
                .collect();
        let domain = crate::infra::repositories::project_domain::domain_for(
            &self.database,
            project,
            crate::infra::node::settings::domain(&self.database)
                .await
                .as_deref(),
        )
        .await;

        let mut members = BTreeSet::from([me.clone()]);
        let mut recipients = BTreeSet::new();
        let mut listed = Vec::new();
        for service in services::all(&self.database, Some(&project.id)).await? {
            let mut copies = Vec::new();
            for replica in replicas::of_service(&self.database, &service.id).await? {
                if replica.evicted() {
                    continue;
                }
                let Some(node_id) = &replica.node_id else {
                    copies.push(Copy {
                        node: me.clone(),
                        slot: replica.slot,
                    });
                    continue;
                };
                let Some(node) = nodes.get(node_id) else {
                    continue;
                };
                let Some(address) = node.overlay_ip.clone() else {
                    continue;
                };
                members.insert(address.clone());
                if errand::opens_doors(node.version.as_deref()) {
                    recipients.insert(node_id.clone());
                    copies.push(Copy {
                        node: address,
                        slot: replica.slot,
                    });
                }
            }
            let mut declared: Vec<u16> = ports::of_service(&self.database, &service.id)
                .await?
                .iter()
                .map(|port| port.container_port)
                .collect();
            declared.sort_unstable();
            declared.dedup();
            if copies.is_empty()
                || declared.is_empty()
                || service.desired_state != DesiredState::Running
            {
                continue;
            }
            let (primary_slot, qualified) =
                self.names_of(project, &service, domain.as_deref()).await?;
            listed.push(Listed {
                service: service.slug.clone(),
                qualified,
                ports: declared,
                primary_slot,
                copies,
            });
        }
        let exits = crate::infra::repositories::egress::exits_of(&self.database, &project.id)
            .await?
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        for exit in exits.iter().filter(|exit| **exit != me) {
            let carries = nodes
                .values()
                .find(|node| node.overlay_ip.as_ref() == Some(exit));
            if let Some(node) =
                carries.filter(|node| errand::enforces_egress(node.version.as_deref()))
            {
                recipients.insert(node.id.clone());
            }
        }
        if members.len() < 2 && !exits.iter().any(|exit| *exit != me) {
            return Ok(None);
        }
        let egress = crate::infra::repositories::egress::of_project(&self.database, &project.id)
            .await?
            .text();
        Ok(Some(Spread {
            directory: Directory {
                project: project.name.clone(),
                members: members.into_iter().collect(),
                services: listed,
                egress: Some(egress),
                exits,
            },
            recipients,
        }))
    }

    /// Tell every node holding a copy of a project of ours what the project
    /// runs where, and every node that was told and holds none any more that
    /// there is nothing. Recomputed on every pass and sent only when it
    /// changed, because what it says arrives from the nodes — a copy's node,
    /// a node's version — and nothing else would send it again.
    pub async fn tell_directories(&self) {
        let Ok(projects) = projects::all(&self.database).await else {
            return;
        };
        for project in projects.iter().filter(|project| project.is_ours()) {
            if let Err(error) = self.tell_directory(project).await {
                tracing::warn!(project = %project.slug, %error, "could not tell the nodes where this project runs");
            }
        }
    }

    pub(crate) async fn tell_directory(&self, project: &Project) -> DeployResult<()> {
        let subject = format!("directory:{}", project.id);
        let nothing = Directory {
            project: project.name.clone(),
            ..Directory::default()
        };
        let (directory, recipients) = match self.spread(project).await? {
            Some(spread) => (spread.directory, spread.recipients),
            None => (nothing.clone(), BTreeSet::new()),
        };
        let said = serde_json::to_value(&directory).expect("a directory is plain data");
        for node in &recipients {
            if errand::queue_if_changed(&self.database, node, Kind::Directory, &subject, &said)
                .await?
                .is_some()
            {
                tracing::info!(project = %project.slug, %node, "told a node where this project runs");
            }
        }
        let withdrawn = serde_json::to_value(&nothing).expect("a directory is plain data");
        for node in errand::told_about(&self.database, &subject).await? {
            if !recipients.contains(&node) {
                errand::queue_if_changed(
                    &self.database,
                    &node,
                    Kind::Directory,
                    &subject,
                    &withdrawn,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// The nodes holding a copy of `project` that are too old to enforce
    /// where it may connect, by name.
    pub async fn too_old_for_egress(&self, project: &Project) -> DeployResult<Vec<String>> {
        let nodes: BTreeMap<String, crate::infra::network::Node> =
            crate::infra::network::all(&self.database)
                .await?
                .into_iter()
                .map(|node| (node.id.clone(), node))
                .collect();
        let mut too_old = BTreeSet::new();
        for service in services::all(&self.database, Some(&project.id)).await? {
            for replica in replicas::of_service(&self.database, &service.id).await? {
                let Some(node) = replica
                    .node_id
                    .as_ref()
                    .filter(|_| !replica.evicted())
                    .and_then(|node_id| nodes.get(node_id))
                else {
                    continue;
                };
                if !errand::enforces_egress(node.version.as_deref()) {
                    too_old.insert(node.name.clone());
                }
            }
        }
        Ok(too_old.into_iter().collect())
    }

    /// What the hosts files of `project` need beyond its own rows: the
    /// services listed only in the directory, and an address for every name
    /// that has no copy here, keyed as `hosts::entries_for` reads them.
    pub(crate) async fn stood_in(
        &self,
        project: &Project,
        here: &BTreeSet<(String, u32)>,
    ) -> DeployResult<StoodIn> {
        let mut stood = StoodIn::default();
        let (Some(directory), Some(me)) =
            (self.directory(project).await?, self.overlay_address().await)
        else {
            return Ok(stood);
        };
        for want in reach::wanted(&directory, &me, here) {
            let host = stand_ins::ensure(&self.database, &project.id, &want.name).await?;
            let Some(listed) = directory
                .services
                .iter()
                .find(|listed| listed.service == want.service)
            else {
                continue;
            };
            let slot = match want.role {
                Role::Any => 0,
                Role::Primary => listed.primary_slot.unwrap_or(0),
                Role::Pool => POOL_SLOT,
            };
            stood.addresses.insert(
                (want.service.clone(), slot),
                reach::stand_in_address(host).to_string(),
            );
            stood.services.push((
                listed.service.clone(),
                listed.primary_slot,
                listed.qualified.clone(),
            ));
        }
        stood.services.dedup();
        Ok(stood)
    }

    /// Whether the caller may reach the copy it knocked for, and where that
    /// copy answers. The caller is asked about by the address the overlay
    /// delivered it from, which a node cannot choose: WireGuard admits a
    /// peer's packets only from the address the peer was given.
    pub(crate) async fn admit(&self, peer: IpAddr, knock: &Knock) -> Result<SocketAddr, String> {
        let project = projects::all(&self.database)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|project| project.name == knock.project)
            .ok_or("no such project here")?;
        let directory = self
            .directory(&project)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("this project runs on no other node")?;
        let peer = peer.to_canonical().to_string();
        if !directory.members.contains(&peer) {
            return Err(format!("{peer} holds nothing of this project"));
        }
        let service = services::in_project(
            &self.database,
            &project.id,
            crate::domain::values::slug::Slug::of(&knock.service).as_str(),
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or("no such service here")?;
        let declared = ports::of_service(&self.database, &service.id)
            .await
            .map_err(|error| error.to_string())?
            .iter()
            .any(|port| port.container_port == knock.port);
        if !declared {
            return Err("a port the service does not declare".into());
        }
        let replica = replicas::of_service(&self.database, &service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|replica| replica.slot == knock.slot && replica.is_here() && !replica.evicted())
            .ok_or("no copy in that slot here")?;
        let address: IpAddr = match replica.reserved_host {
            Some(host) => {
                let index = projects::network_index(&self.database, &project.id)
                    .await
                    .map_err(|error| error.to_string())?
                    .ok_or("the project has no network here")?;
                ProjectNetwork::new(index)
                    .and_then(|net| net.reserved_address(host))
                    .map_err(|error| error.to_string())?
                    .into()
            }
            None => replica
                .address
                .as_deref()
                .ok_or("that copy is not running")?
                .parse()
                .map_err(|_| "that copy's address is not one")?,
        };
        Ok(SocketAddr::new(address, knock.port))
    }

    /// Where a container's connection to a stand-in may go: the doors of the
    /// nodes running a copy, in the order to try them.
    ///
    /// The project is the bridge the connection came in on, which is the
    /// address the `REDIRECT` delivered it to — the kernel's choice, not the
    /// container's. The caller has to be on that bridge as well, so a
    /// source address written by hand does not name another project.
    pub(crate) async fn route(
        &self,
        peer: SocketAddrV4,
        arrived: Ipv4Addr,
        asked: SocketAddrV4,
    ) -> Result<Vec<Route>, String> {
        let [10, 42, index, _] = arrived.octets() else {
            return Err("not through a project's bridge".into());
        };
        let net = ProjectNetwork::new(index).map_err(|error| error.to_string())?;
        if arrived.to_string() != net.gateway() || !net.holds(*peer.ip()) {
            return Err("not from the bridge it came in on".into());
        }
        let host = reach::stand_in_host(*asked.ip()).ok_or("not a stand-in")?;
        let project = projects::with_network_index(&self.database, index)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("no project on that bridge")?;
        let name = stand_ins::name_at(&self.database, &project.id, host)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("no name stands in there")?;
        let directory = self
            .directory(&project)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("this project runs on no other node")?;
        let (listed, role) = reach::resolve(&directory, &name).ok_or("a name no longer listed")?;
        if !listed.ports.contains(&asked.port()) {
            return Err("a port the service does not declare".into());
        }
        let me = self.overlay_address().await.unwrap_or_default();
        let mut copies = reach::candidates(listed, role, &me);
        if !copies.is_empty() {
            let start = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % copies.len();
            copies.rotate_left(start);
        }
        Ok(copies
            .into_iter()
            .filter_map(|copy| {
                let node: Ipv4Addr = copy.node.parse().ok()?;
                Some(Route {
                    door: SocketAddr::new(node.into(), reach::DOOR_PORT),
                    knock: Knock {
                        project: directory.project.clone(),
                        service: listed.service.clone(),
                        slot: copy.slot,
                        port: asked.port(),
                    },
                })
            })
            .collect())
    }
}

/// Where the next connection to a name with several copies starts.
static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// See [`Deployer::stood_in`].
#[derive(Debug, Default)]
pub(crate) struct StoodIn {
    pub services: Vec<(String, Option<u32>, Option<String>)>,
    pub addresses: BTreeMap<(String, u32), String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::service::Service;
    use crate::domain::values::destinations::Destinations;
    use crate::infra::config::Config;
    use crate::infra::network::Node;
    use crate::infra::repositories::egress;
    use wabot::sqlite::SqliteDatabase;

    async fn node(database: &SqliteDatabase, id: &str, overlay: &str, version: &str) {
        crate::infra::network::save(
            database,
            &Node {
                update_note: None,
                version: Some(version.into()),
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: id.into(),
                name: format!("{id}.example"),
                kind: crate::infra::network::Kind::Private,
                overlay_ip: Some(overlay.into()),
                public_key: None,
                endpoint: None,
                allows: vec![crate::domain::values::capability::Capability::Host],
                ca_pem: None,
                last_seen_at: None,
                is_self: false,
            },
        )
        .await
        .expect("node");
    }

    async fn service(
        database: &SqliteDatabase,
        project: &Project,
        name: &str,
        port: u16,
        on: Option<&str>,
    ) -> Service {
        let service = services::create(database, &project.id, name, "registry.example/x:1", &[])
            .await
            .expect("service");
        ports::create(database, &service.id, port, false, None, &[])
            .await
            .expect("port");
        if let Some(node) = on {
            let copy = replicas::of_service(database, &service.id)
                .await
                .expect("replicas")
                .pop()
                .expect("a service is created with one copy");
            replicas::move_to(database, &copy.id, Some(node))
                .await
                .expect("moved");
        }
        service
    }

    /// The owner, with `web` here and `api` on `nd-far`.
    async fn spanning(far_version: &str) -> (SqliteDatabase, Deployer, Project, String) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let me = crate::infra::network::ensure_hub(&database, &Config::default())
            .await
            .expect("hub")
            .overlay_ip
            .expect("an overlay address");
        node(&database, "nd-far", "10.42.0.9", far_version).await;
        let project = projects::create(&database, "shop").await.expect("project");
        service(&database, &project, "web", 8080, None).await;
        service(&database, &project, "api", 9000, Some("nd-far")).await;
        let deployer = Deployer::new(std::sync::Arc::new(database.clone()), &Config::default());
        (database, deployer, project, me)
    }

    async fn told(database: &SqliteDatabase, node: &str) -> Vec<Directory> {
        errand::waiting(database, node)
            .await
            .expect("errands")
            .into_iter()
            .filter(|errand| errand.kind == Kind::Directory)
            .map(|errand| serde_json::from_value(errand.payload).expect("a directory"))
            .collect()
    }

    #[tokio::test]
    async fn a_project_on_one_node_has_no_directory_and_tells_nobody() {
        let (database, deployer, project, _) = spanning("0.44.0").await;
        let api = services::in_project(&database, &project.id, "api")
            .await
            .expect("query")
            .expect("api");
        let copy = replicas::of_service(&database, &api.id)
            .await
            .expect("replicas")
            .pop()
            .expect("its copy");
        replicas::move_to(&database, &copy.id, None)
            .await
            .expect("brought home");

        assert_eq!(deployer.directory(&project).await.expect("directory"), None);
        deployer.tell_directories().await;
        assert!(told(&database, "nd-far").await.is_empty());
    }

    /// The directory is derived from what the nodes report, so it is sent
    /// again when it changes and not otherwise — and a node that holds
    /// nothing any more is told so, or it would keep opening its door to
    /// the project for ever.
    #[tokio::test]
    async fn the_owner_tells_each_node_holding_a_copy_where_the_project_runs() {
        let (database, deployer, project, me) = spanning("0.44.0").await;

        deployer.tell_directories().await;
        deployer.tell_directories().await;

        let directory = told(&database, "nd-far").await;
        assert_eq!(directory.len(), 1, "told once: {directory:?}");
        let directory = &directory[0];
        assert_eq!(directory.project, "shop");
        let mut members = vec![me.clone(), "10.42.0.9".to_string()];
        members.sort();
        assert_eq!(directory.members, members);
        let where_runs = |name: &str| {
            directory
                .services
                .iter()
                .find(|listed| listed.service == name)
                .map(|listed| (listed.ports.clone(), listed.copies.clone()))
        };
        assert_eq!(
            where_runs("web"),
            Some((
                vec![8080],
                vec![Copy {
                    node: me.clone(),
                    slot: 1
                }]
            ))
        );
        assert_eq!(
            where_runs("api"),
            Some((
                vec![9000],
                vec![Copy {
                    node: "10.42.0.9".into(),
                    slot: 1
                }]
            ))
        );

        let api = services::in_project(&database, &project.id, "api")
            .await
            .expect("query")
            .expect("api");
        let copy = replicas::of_service(&database, &api.id)
            .await
            .expect("replicas")
            .pop()
            .expect("its copy");
        replicas::move_to(&database, &copy.id, None)
            .await
            .expect("brought home");
        deployer.tell_directories().await;

        let after = told(&database, "nd-far").await;
        assert_eq!(after.len(), 2, "{after:?}");
        assert_eq!(
            after[1],
            Directory {
                project: "shop".into(),
                ..Directory::default()
            },
            "a node holding nothing of the project is told it is in nothing"
        );
    }

    #[tokio::test]
    async fn a_node_holding_a_copy_is_told_again_where_the_project_may_connect() {
        let (database, deployer, project, _) = spanning("0.44.0").await;
        deployer.tell_directories().await;
        let list = Destinations::parse("203.0.113.0/24:443").expect("a list");
        egress::set(&database, &project.id, &list)
            .await
            .expect("set");
        deployer.tell_directories().await;

        let lists: Vec<Option<String>> = told(&database, "nd-far")
            .await
            .into_iter()
            .map(|directory| directory.egress)
            .collect();
        assert_eq!(
            lists,
            vec![
                Some("0.0.0.0/0".to_string()),
                Some("203.0.113.0/24:443".to_string())
            ]
        );
    }

    /// An exit holds no copy, so nothing about the copies would ever have
    /// sent it the project — and it is the one node that cannot carry
    /// anybody out without knowing who they are.
    #[tokio::test]
    async fn a_project_on_this_node_alone_still_tells_the_exits_it_leaves_through() {
        let (database, deployer, project, me) = spanning("0.44.0").await;
        node(&database, "nd-exit", "10.42.0.7", "0.44.0").await;
        node(&database, "nd-old-exit", "10.42.0.8", "0.43.0").await;
        let api = services::in_project(&database, &project.id, "api")
            .await
            .expect("query")
            .expect("api");
        let copy = replicas::of_service(&database, &api.id)
            .await
            .expect("replicas")
            .pop()
            .expect("its copy");
        replicas::move_to(&database, &copy.id, None)
            .await
            .expect("brought home");
        egress::set_exits(
            &database,
            &project.id,
            &[
                "10.42.0.7".parse().expect("ip"),
                "10.42.0.8".parse().expect("ip"),
            ],
        )
        .await
        .expect("exits");

        deployer.tell_directories().await;

        let told_exit = told(&database, "nd-exit").await;
        assert_eq!(told_exit.len(), 1, "{told_exit:?}");
        assert_eq!(told_exit[0].members, vec![me]);
        assert_eq!(told_exit[0].exits, vec!["10.42.0.7", "10.42.0.8"]);
        assert!(told(&database, "nd-far").await.is_empty());
        assert!(told(&database, "nd-old-exit").await.is_empty());

        egress::set_exits(&database, &project.id, &[])
            .await
            .expect("exits");
        deployer.tell_directories().await;
        let withdrawn = told(&database, "nd-exit").await;
        assert!(
            withdrawn.last().is_some_and(|said| said.members.is_empty()),
            "an exit no longer chosen is told there is nothing: {withdrawn:?}"
        );
    }

    #[tokio::test]
    async fn a_node_too_old_to_enforce_a_list_is_named_while_it_holds_a_copy() {
        let (database, deployer, project, _) = spanning("0.43.0").await;
        assert_eq!(
            deployer.too_old_for_egress(&project).await.expect("read"),
            vec!["nd-far.example".to_string()]
        );

        node(&database, "nd-far", "10.42.0.9", "0.44.0").await;
        assert!(deployer
            .too_old_for_egress(&project)
            .await
            .expect("read")
            .is_empty());
    }

    #[tokio::test]
    async fn a_node_too_old_to_have_a_door_is_neither_told_nor_sent_to() {
        let (database, deployer, project, _) = spanning("0.43.0").await;

        deployer.tell_directories().await;

        assert!(told(&database, "nd-far").await.is_empty());
        let directory = deployer
            .directory(&project)
            .await
            .expect("directory")
            .expect("the project still spans two nodes");
        assert!(
            directory
                .services
                .iter()
                .all(|listed| listed.service != "api"),
            "a copy behind no door is no route: {directory:?}"
        );
    }

    #[tokio::test]
    async fn the_door_lets_in_a_node_holding_the_project_to_a_declared_port_of_a_copy_here() {
        let (database, deployer, project, _) = spanning("0.44.0").await;
        let web = services::in_project(&database, &project.id, "web")
            .await
            .expect("query")
            .expect("web");
        let copy = replicas::in_slot(&database, &web.id, 1)
            .await
            .expect("query")
            .expect("its copy");
        replicas::set_address(&database, &copy.id, Some("10.42.3.5"))
            .await
            .expect("address");
        let knock = |service: &str, slot: u32, port: u16| Knock {
            project: "shop".into(),
            service: service.into(),
            slot,
            port,
        };
        let far: IpAddr = "10.42.0.9".parse().expect("ip");

        assert_eq!(
            deployer.admit(far, &knock("web", 1, 8080)).await,
            Ok("10.42.3.5:8080".parse().expect("addr"))
        );
        let stranger: IpAddr = "10.42.0.7".parse().expect("ip");
        assert!(deployer
            .admit(stranger, &knock("web", 1, 8080))
            .await
            .expect_err("a node holding nothing of it")
            .contains("holds nothing"));
        assert!(deployer.admit(far, &knock("web", 1, 22)).await.is_err());
        assert!(deployer.admit(far, &knock("web", 2, 8080)).await.is_err());
        assert!(
            deployer.admit(far, &knock("api", 1, 9000)).await.is_err(),
            "its copy is on the caller, not here"
        );
        let mut elsewhere = knock("web", 1, 8080);
        elsewhere.project = "other".into();
        assert!(deployer.admit(far, &elsewhere).await.is_err());
    }

    /// Through a real socket: the knock, the answer, and what the client
    /// says after it arriving at the copy unchanged — and a caller the
    /// directory does not name hearing nothing at all.
    #[tokio::test]
    async fn a_member_reaches_the_copy_through_the_door_and_a_stranger_does_not() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let copy_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let copy_port = copy_listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = copy_listener.accept().await {
                tokio::spawn(async move {
                    let mut buffer = [0u8; 64];
                    while let Ok(read) = stream.read(&mut buffer).await {
                        if read == 0 || stream.write_all(&buffer[..read]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });

        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::network::ensure_hub(&database, &Config::default())
            .await
            .expect("hub");
        node(&database, "nd-far", "127.0.0.1", "0.44.0").await;
        let project = projects::create(&database, "shop").await.expect("project");
        let web = service(&database, &project, "web", copy_port, None).await;
        service(&database, &project, "api", 9000, Some("nd-far")).await;
        let copy = replicas::in_slot(&database, &web.id, 1)
            .await
            .expect("query")
            .expect("its copy");
        replicas::set_address(&database, &copy.id, Some("127.0.0.1"))
            .await
            .expect("address");
        let deployer = Deployer::new(std::sync::Arc::new(database.clone()), &Config::default());

        let door = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let door_address = door.local_addr().expect("addr");
        tokio::spawn({
            let deployer = deployer.clone();
            async move {
                while let Ok((caller, peer)) = door.accept().await {
                    let deployer = deployer.clone();
                    tokio::spawn(async move {
                        crate::infra::reach::door::open(&deployer, caller, peer).await
                    });
                }
            }
        });
        let knock = Knock {
            project: "shop".into(),
            service: "web".into(),
            slot: 1,
            port: copy_port,
        };

        let mut member = tokio::net::TcpStream::connect(door_address)
            .await
            .expect("connect");
        member.write_all(&knock.line()).await.expect("knock");
        member.write_all(b"hello").await.expect("hello");
        let mut heard = [0u8; 8];
        member.read_exact(&mut heard).await.expect("answer");
        assert_eq!(&heard, b"ok\nhello");

        node(&database, "nd-far", "10.42.0.9", "0.44.0").await;
        let mut stranger = tokio::net::TcpStream::connect(door_address)
            .await
            .expect("connect");
        stranger.write_all(&knock.line()).await.expect("knock");
        let mut heard = [0u8; 3];
        let answer =
            tokio::time::timeout(std::time::Duration::from_secs(5), stranger.read(&mut heard))
                .await
                .expect("the door hangs up on a stranger rather than holding the line");
        assert!(
            matches!(answer, Ok(0) | Err(_)),
            "a stranger heard {:?}",
            &heard[..answer.unwrap_or(0)]
        );
    }

    /// The case that started this: a service here, its database's primary
    /// on the node that owns the project, and no name for it in here.
    #[tokio::test]
    async fn a_database_on_another_node_resolves_here_and_routes_to_its_door() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let me = crate::infra::network::ensure_hub(&database, &Config::default())
            .await
            .expect("hub")
            .overlay_ip
            .expect("an overlay address");
        let project = projects::create(&database, "shop").await.expect("project");
        projects::set_origin(&database, &project.id, "nd-owner")
            .await
            .expect("origin");
        let project = projects::find(&database, &project.id)
            .await
            .expect("query")
            .expect("project");
        let index = projects::ensure_network_index(&database, &project.id)
            .await
            .expect("a network");
        directories::keep(
            &database,
            &project.id,
            &Directory {
                project: "shop".into(),
                members: vec![me.clone(), "10.42.0.9".into()],
                services: vec![Listed {
                    service: "orders".into(),
                    qualified: Some("orders.shop.example".into()),
                    ports: vec![5432],
                    primary_slot: Some(1),
                    copies: vec![Copy {
                        node: "10.42.0.9".into(),
                        slot: 1,
                    }],
                }],
                egress: None,
                exits: Vec::new(),
            },
        )
        .await
        .expect("kept");
        let deployer = Deployer::new(std::sync::Arc::new(database.clone()), &Config::default());
        let here = BTreeSet::from([("web".to_string(), 1)]);

        let stood = deployer.stood_in(&project, &here).await.expect("stood in");
        let again = deployer.stood_in(&project, &here).await.expect("stood in");
        assert_eq!(stood.addresses, again.addresses, "a name keeps its address");

        let mut addresses: BTreeMap<(String, u32), String> =
            BTreeMap::from([(("web".into(), 1), "10.42.3.5".into())]);
        addresses.extend(stood.addresses);
        let mut named = vec![("web".to_string(), None, None)];
        named.extend(stood.services);
        let entries = super::super::hosts::entries_for(&named, &addresses, "shop", "web");
        let resolved = |name: &str| {
            entries
                .iter()
                .find(|entry| entry.names.iter().any(|n| n == name))
                .map(|entry| entry.addresses.clone())
        };
        assert_eq!(resolved("orders"), Some(vec!["10.42.255.1".to_string()]));
        assert_eq!(
            resolved("orders.shop.example"),
            Some(vec!["10.42.255.1".to_string()])
        );
        assert_eq!(resolved("orders-ro"), Some(vec!["10.42.255.2".to_string()]));

        let from = SocketAddrV4::new(Ipv4Addr::new(10, 42, index, 5), 40000);
        let gateway = Ipv4Addr::new(10, 42, index, 1);
        let to = |host: u8, port: u16| SocketAddrV4::new(Ipv4Addr::new(10, 42, 255, host), port);
        assert_eq!(
            deployer.route(from, gateway, to(1, 5432)).await,
            Ok(vec![Route {
                door: "10.42.0.9:31101".parse().expect("addr"),
                knock: Knock {
                    project: "shop".into(),
                    service: "orders".into(),
                    slot: 1,
                    port: 5432,
                },
            }])
        );
        assert!(deployer.route(from, gateway, to(1, 22)).await.is_err());
        assert!(deployer.route(from, gateway, to(9, 5432)).await.is_err());
        let other = Ipv4Addr::new(10, 42, index + 1, 1);
        let other_bridge = SocketAddrV4::new(Ipv4Addr::new(10, 42, index + 1, 5), 40000);
        assert!(
            deployer
                .route(other_bridge, other, to(1, 5432))
                .await
                .is_err(),
            "another project's container asking for the same stand-in"
        );
        assert!(
            deployer.route(from, other, to(1, 5432)).await.is_err(),
            "an address on one bridge arriving through another"
        );
    }
}
