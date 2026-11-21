//! What the projects' resolver asks of the node.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::{usable_nameservers, Deployer, ProjectNetwork};
use crate::domain::values::capability::Capability;
use crate::domain::values::destinations::{is_internet, Destinations};
use crate::infra::clock::now_ms;
use crate::infra::egress::OPEN_AT_LEAST;
use crate::infra::reach::{Leave, DOOR_PORT};
use crate::infra::repositories::{egress, projects};

/// Where the next connection leaving through several exits starts.
static NEXT_EXIT: AtomicUsize = AtomicUsize::new(0);

/// A node this node's projects may leave through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitCandidate {
    pub node_id: String,
    pub name: String,
    pub address: Ipv4Addr,
    pub is_self: bool,
}

impl Deployer {
    /// The project whose bridge `source` is on, by id, and its list.
    pub(crate) async fn egress_from(&self, source: Ipv4Addr) -> Option<(String, Destinations)> {
        let index = source.octets()[2];
        let network = ProjectNetwork::new(index).ok()?;
        if !network.holds(source) || source.to_string() == network.gateway() {
            return None;
        }
        let project = projects::with_network_index(&self.database, index)
            .await
            .ok()??;
        let destinations = egress::of_project(&self.database, &project.id).await.ok()?;
        Some((project.id, destinations))
    }

    /// Where the containers' own queries would have gone.
    pub(crate) fn upstreams(&self) -> Vec<SocketAddr> {
        let written = std::fs::read_to_string(&self.resolv_conf).unwrap_or_default();
        usable_nameservers(&written)
            .iter()
            .filter_map(|server| server.parse::<Ipv4Addr>().ok())
            .map(|server| SocketAddr::from((server, 53)))
            .collect()
    }

    /// Open what a name on the list resolved to, and tell the kernel before
    /// the answer goes back when anything was not open already.
    pub(crate) async fn open_learned(
        &self,
        project_id: &str,
        name: &str,
        addresses: &[Ipv4Addr],
        ports: &[Option<u16>],
        cached: Duration,
    ) {
        if addresses.is_empty() {
            return;
        }
        let opened: Vec<(Ipv4Addr, Option<u16>)> = addresses
            .iter()
            .flat_map(|address| ports.iter().map(move |port| (*address, *port)))
            .collect();
        let now = now_ms();
        let open_for = cached.max(OPEN_AT_LEAST).as_millis() as i64;
        match egress::learn(
            &self.database,
            project_id,
            name,
            &opened,
            now + open_for,
            now,
        )
        .await
        {
            Ok(true) => self.keep_projects_apart(false).await,
            Ok(false) => {}
            Err(error) => tracing::warn!(%error, %name, "could not open what a name resolved to"),
        }
    }

    pub(crate) async fn note_refused(&self, project_id: &str, name: &str) {
        if let Err(error) = egress::refuse(&self.database, project_id, name, now_ms()).await {
            tracing::warn!(%error, %name, "could not record a refused name");
        }
    }

    /// Forget what has run out. The kernel already has: every pass renders
    /// only what is in date.
    pub async fn forget_expired_egress(&self) {
        if let Err(error) = egress::forget_expired(&self.database, now_ms()).await {
            tracing::warn!(%error, "could not forget what the projects' names opened");
        }
    }

    /// Whether the caller may leave through this node for `leave.to`, and
    /// where that is. The source has already judged the destination against
    /// the project's list; this node judges who is leaving through it.
    pub(crate) async fn admit_leaving(
        &self,
        peer: IpAddr,
        leave: &Leave,
    ) -> Result<SocketAddr, String> {
        let project = projects::all(&self.database)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|project| project.name == leave.project)
            .ok_or("no such project here")?;
        let me = self
            .overlay_ip()
            .await
            .ok_or("this node is in no network")?;
        let exits = egress::exits_of(&self.database, &project.id)
            .await
            .map_err(|error| error.to_string())?;
        if !exits.contains(&me) {
            return Err("this node is not one of the project's exits".into());
        }
        if let Some(origin) = &project.origin_node_id {
            let granted =
                crate::infra::network::capability::granted_to(&self.database, origin).await;
            if !granted.contains(&Capability::Exit) {
                return Err(format!("this node does not carry {origin}'s projects"));
            }
        }
        let directory = self
            .directory(&project)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("this project runs on no other node")?;
        let peer = peer.to_canonical().to_string();
        if !directory.members.contains(&peer) {
            return Err(format!("{peer} holds nothing of this project"));
        }
        if !is_internet(*leave.to.ip()) {
            return Err(format!("{} is not the internet", leave.to));
        }
        Ok(SocketAddr::V4(leave.to))
    }

    /// The exits a container's connection to `to` is carried to, starting
    /// at a different one each time, and what to say at their doors.
    pub(crate) async fn exits_for(
        &self,
        peer: SocketAddrV4,
        arrived: Ipv4Addr,
        to: SocketAddrV4,
    ) -> Result<(Leave, Vec<SocketAddr>), String> {
        let index = peer.ip().octets()[2];
        let network = ProjectNetwork::new(index).map_err(|error| error.to_string())?;
        if arrived.to_string() != network.gateway() || !network.holds(*peer.ip()) {
            return Err("not from a project's own bridge".into());
        }
        if !is_internet(*to.ip()) {
            return Err(format!("{to} is not the internet"));
        }
        let project = projects::with_network_index(&self.database, index)
            .await
            .map_err(|error| error.to_string())?
            .ok_or("no project on that bridge")?;
        let me = self.overlay_ip().await;
        let mut exits: Vec<Ipv4Addr> = egress::exits_of(&self.database, &project.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|exit| Some(*exit) != me)
            .collect();
        if exits.is_empty() {
            return Err("this project leaves through this node".into());
        }
        let start = NEXT_EXIT.fetch_add(1, Ordering::Relaxed) % exits.len();
        exits.rotate_left(start);
        Ok((
            Leave {
                project: project.name,
                to,
            },
            exits
                .into_iter()
                .map(|exit| SocketAddr::new(exit.into(), DOOR_PORT))
                .collect(),
        ))
    }

    /// This node, when it is in a network, and every node that lets it name
    /// that node as an exit and is new enough to be one.
    pub async fn exit_candidates(&self) -> super::DeployResult<Vec<ExitCandidate>> {
        let mut candidates = Vec::new();
        for node in crate::infra::network::all(&self.database).await? {
            let Some(address) = node.overlay_ip.as_deref().and_then(|ip| ip.parse().ok()) else {
                continue;
            };
            let carries = node.is_self
                || (node.allows.contains(&Capability::Exit)
                    && crate::infra::network::errand::enforces_egress(node.version.as_deref()));
            if carries {
                candidates.push(ExitCandidate {
                    node_id: node.id,
                    name: node.name,
                    address,
                    is_self: node.is_self,
                });
            }
        }
        candidates.sort_by_key(|candidate| (!candidate.is_self, candidate.name.clone()));
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::project::Project;
    use crate::infra::config::Config;
    use crate::infra::network::errand::Directory;
    use wabot::sqlite::SqliteDatabase;

    fn at(address: &str) -> Ipv4Addr {
        address.parse().expect("an address")
    }

    /// This node, an exit for `nd-owner`'s project `shop`, which runs on
    /// `10.42.0.9`.
    async fn exit() -> (SqliteDatabase, Deployer, Project, Ipv4Addr) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let me = at(
            &crate::infra::network::ensure_hub(&database, &Config::default())
                .await
                .expect("hub")
                .overlay_ip
                .expect("an overlay address"),
        );
        crate::infra::network::capability::set_provides(&database, Capability::Exit, true)
            .await
            .expect("provided");
        crate::infra::network::capability::grant(&database, "nd-owner", &[Capability::Exit])
            .await
            .expect("granted");
        let project = projects::create(&database, "shop").await.expect("project");
        projects::set_origin(&database, &project.id, "nd-owner")
            .await
            .expect("origin");
        let project = projects::all(&database)
            .await
            .expect("query")
            .into_iter()
            .find(|found| found.id == project.id)
            .expect("the project");
        crate::infra::repositories::directories::keep(
            &database,
            &project.id,
            &Directory {
                project: "shop".into(),
                members: vec!["10.42.0.9".into()],
                services: Vec::new(),
                egress: Some("0.0.0.0/0".into()),
                exits: vec![me.to_string()],
            },
        )
        .await
        .expect("kept");
        egress::set_exits(&database, &project.id, &[me])
            .await
            .expect("exits");
        let deployer = Deployer::new(std::sync::Arc::new(database.clone()), &Config::default());
        (database, deployer, project, me)
    }

    fn leave(to: &str) -> Leave {
        Leave {
            project: "shop".into(),
            to: to.parse().expect("an address"),
        }
    }

    #[tokio::test]
    async fn an_exit_carries_a_node_holding_the_project_to_the_internet() {
        let (_, deployer, _, _) = exit().await;
        assert_eq!(
            deployer
                .admit_leaving(IpAddr::V4(at("10.42.0.9")), &leave("140.82.114.6:443"))
                .await,
            Ok("140.82.114.6:443".parse().expect("an address"))
        );
    }

    #[tokio::test]
    async fn an_exit_carries_nobody_else_and_nowhere_that_is_not_the_internet() {
        let (database, deployer, project, _) = exit().await;
        let member = IpAddr::V4(at("10.42.0.9"));

        for (peer, to, why) in [
            (
                IpAddr::V4(at("10.42.0.5")),
                "140.82.114.6:443",
                "holds nothing",
            ),
            (member, "192.168.104.1:22", "not the internet"),
            (member, "169.254.169.254:80", "not the internet"),
            (member, "10.42.1.5:5432", "not the internet"),
        ] {
            let refused = deployer.admit_leaving(peer, &leave(to)).await;
            assert!(
                refused.as_ref().is_err_and(|reason| reason.contains(why)),
                "{peer} to {to}: {refused:?}"
            );
        }

        let mut other = leave("140.82.114.6:443");
        other.project = "other".into();
        assert!(deployer.admit_leaving(member, &other).await.is_err());

        egress::set_exits(&database, &project.id, &[at("10.42.0.7")])
            .await
            .expect("exits");
        let refused = deployer
            .admit_leaving(member, &leave("140.82.114.6:443"))
            .await;
        assert!(
            refused
                .as_ref()
                .is_err_and(|reason| reason.contains("not one of the project's exits")),
            "{refused:?}"
        );
    }

    /// The grant is the exit operator's decision and the more recent one:
    /// a directory kept from before it was withdrawn carries nobody.
    #[tokio::test]
    async fn an_exit_that_stopped_carrying_for_the_owner_lets_nobody_out() {
        let (database, deployer, _, _) = exit().await;
        crate::infra::network::capability::set_granted(&database, "nd-owner", &[])
            .await
            .expect("withdrawn");
        let refused = deployer
            .admit_leaving(IpAddr::V4(at("10.42.0.9")), &leave("140.82.114.6:443"))
            .await;
        assert!(
            refused
                .as_ref()
                .is_err_and(|reason| reason.contains("does not carry")),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn a_connection_leaving_is_carried_to_every_other_exit_starting_somewhere_new() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let me = at(
            &crate::infra::network::ensure_hub(&database, &Config::default())
                .await
                .expect("hub")
                .overlay_ip
                .expect("an overlay address"),
        );
        let project = projects::create(&database, "shop").await.expect("project");
        let index = projects::ensure_network_index(&database, &project.id)
            .await
            .expect("a network");
        let network = ProjectNetwork::new(index).expect("a network");
        let gateway: Ipv4Addr = network.gateway().parse().expect("an address");
        let container = SocketAddrV4::new(Ipv4Addr::new(10, 42, index, 5), 40000);
        let to: SocketAddrV4 = "140.82.114.6:443".parse().expect("an address");
        let deployer = Deployer::new(std::sync::Arc::new(database.clone()), &Config::default());

        egress::set_exits(&database, &project.id, &[me])
            .await
            .expect("exits");
        assert!(deployer.exits_for(container, gateway, to).await.is_err());

        egress::set_exits(
            &database,
            &project.id,
            &[me, at("10.42.0.7"), at("10.42.0.8")],
        )
        .await
        .expect("exits");
        let (leave, first) = deployer
            .exits_for(container, gateway, to)
            .await
            .expect("exits");
        assert_eq!(leave.project, "shop");
        assert_eq!(leave.to, to);
        let doors: Vec<SocketAddr> = ["10.42.0.7:31101", "10.42.0.8:31101"]
            .iter()
            .map(|door| door.parse().expect("an address"))
            .collect();
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(sorted, doors, "this node is never its own exit's door");
        let (_, second) = deployer
            .exits_for(container, gateway, to)
            .await
            .expect("exits");
        assert_ne!(
            first[0], second[0],
            "the next connection starts at the other exit"
        );

        for (peer, arrived, to) in [
            (container, gateway, "192.168.1.10:443"),
            (container, at("10.42.200.1"), "140.82.114.6:443"),
        ] {
            assert!(
                deployer
                    .exits_for(peer, arrived, to.parse().expect("an address"))
                    .await
                    .is_err(),
                "{peer} at {arrived} to {to}"
            );
        }
    }
}
