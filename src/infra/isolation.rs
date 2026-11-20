//! Projects kept apart by the kernel.
//!
//! ## A bridge each was never the separation
//!
//! Every project has its own bridge, and a bridge is a separate L2
//! domain and nothing more. The node forwards — a container needs it to
//! reach anything past its bridge — and `FORWARD` accepted everything, so
//! a container reached every other project's containers by address,
//! through the node. Measured on node-1 on 2026-08-25 and again on the VM
//! on 2026-10-15, from an organisation's container to the operator's
//! nginx and Postgres (`docs/vm-walk.md` §2.0). `wabot0` is on the same
//! machine, so a container reached the overlay as well: every other
//! node's ports for its copies, and through them containers of any
//! project on any node.
//!
//! ## What crosses
//!
//! - Anything inside one bridge, and every reply.
//! - A port published on one of the node's own addresses outside
//!   `10.42.0.0/16`, from another project as from anywhere: it is a
//!   public door, and the list guarding it (`firewall`) judges a
//!   container's address as it judges anybody's. The same port reached at
//!   a bridge's gateway or at the overlay address is not that door — it
//!   is the way into another project's copy that a public door would
//!   have hidden.
//! - From the overlay, only what a port bound on this node's overlay
//!   address sent on: an edge elsewhere, a standby elsewhere, the owner
//!   carrying a published database. A container's own address is never
//!   reached from another node.
//! - Into the overlay, only a standby following its primary on another
//!   node: from that project's bridge, to that node, on that port. The
//!   port belongs to one copy of one project, so it cannot be used to
//!   reach any other.
//! - A name whose copies all run elsewhere: a connection to its stand-in
//!   in `10.42.255.0/24` is redirected to the node itself, which carries
//!   it to the door of a node running a copy (`infra::reach`). The node
//!   makes that second connection, so nothing from a bridge reaches the
//!   overlay by this way either, and the door decides who is let in.
//!
//! A container reaches the node itself at its own gateway and at the
//! node's addresses outside `10.42.0.0/16`, as anything outside does.
//! Another bridge's gateway and the overlay address are other networks.
//!
//! ## It only drops
//!
//! What crosses goes back to `FORWARD` and `INPUT` and to whatever else
//! the machine keeps there. The node decides what passes between the
//! networks it made, and nothing about anybody else's.
//!
//! ## Written whole, and only where armed
//!
//! As `firewall`, and for its reasons: one `iptables-restore --noflush`
//! that empties both chains and writes them again, so there is no moment
//! with half the rules, and nothing at all unless `serve` armed it — a
//! test runs as root on the VM node.

use std::net::Ipv4Addr;

use wabot::sqlite::SqliteDatabase;

use crate::infra::network::tunnel::INTERFACE as OVERLAY;
use crate::infra::runtime::network::{ProjectNetwork, BRIDGE_PREFIX, SPACE};

pub const BETWEEN: &str = "WABOT-PROJECTS";
pub const INTO_THE_NODE: &str = "WABOT-PROJECTS-IN";
pub const TO_THE_NAMES: &str = "WABOT-PROJECTS-NAMES";

const ERROR: &str = "isolation.last_error";

/// A standby on this node, and the port it follows its primary on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Followed {
    pub index: u8,
    pub address: Ipv4Addr,
    pub port: u16,
}

/// What the rules are written from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub bridges: Vec<u8>,
    pub followed: Vec<Followed>,
}

/// The input `iptables-restore --noflush` is given.
pub fn render(plan: &Plan) -> String {
    let every_bridge = format!("{BRIDGE_PREFIX}+");
    let networks: Vec<ProjectNetwork> = plan
        .bridges
        .iter()
        .filter_map(|index| ProjectNetwork::new(*index).ok())
        .collect();

    let mut rules = vec![
        "*filter".to_string(),
        format!(":{BETWEEN} - [0:0]"),
        format!(":{INTO_THE_NODE} - [0:0]"),
        format!("-A {BETWEEN} -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN"),
    ];
    for network in &networks {
        rules.push(format!(
            "-A {BETWEEN} -i {bridge} -o {bridge} -j RETURN",
            bridge = network.bridge
        ));
    }
    rules.push(format!(
        "-A {BETWEEN} -i {every_bridge} -o {every_bridge} -m conntrack --ctstate DNAT \
         ! --ctorigdst {SPACE} -j RETURN"
    ));
    rules.push(format!(
        "-A {BETWEEN} -i {every_bridge} -o {every_bridge} \
         -m comment --comment \"wabot-deploy another project\" -j DROP"
    ));
    for followed in &plan.followed {
        let Ok(network) = ProjectNetwork::new(followed.index) else {
            continue;
        };
        rules.push(format!(
            "-A {BETWEEN} -i {} -o {OVERLAY} -d {}/32 -p tcp --dport {} -j RETURN",
            network.bridge, followed.address, followed.port
        ));
    }
    rules.push(format!(
        "-A {BETWEEN} -i {every_bridge} -o {OVERLAY} \
         -m comment --comment \"wabot-deploy the overlay\" -j DROP"
    ));
    rules.push(format!(
        "-A {BETWEEN} -i {OVERLAY} -o {every_bridge} -m conntrack --ctstate DNAT -j RETURN"
    ));
    rules.push(format!(
        "-A {BETWEEN} -i {OVERLAY} -o {every_bridge} \
         -m comment --comment \"wabot-deploy a container from the overlay\" -j DROP"
    ));
    rules.push(format!(
        "-A {BETWEEN} -i {every_bridge} -d {stand_ins} \
         -m comment --comment \"wabot-deploy a stand-in nothing listens at\" -j DROP",
        stand_ins = crate::infra::reach::STAND_INS
    ));

    rules.push(format!(
        "-A {INTO_THE_NODE} -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN"
    ));
    for network in &networks {
        rules.push(format!(
            "-A {INTO_THE_NODE} -i {} -d {} -j RETURN",
            network.bridge,
            network.subnet()
        ));
    }
    rules.push(format!(
        "-A {INTO_THE_NODE} -i {every_bridge} -d {SPACE} \
         -m comment --comment \"wabot-deploy another network on this node\" -j DROP"
    ));
    rules.push("COMMIT".to_string());

    rules.extend([
        "*nat".to_string(),
        format!(":{TO_THE_NAMES} - [0:0]"),
        format!(
            "-A {TO_THE_NAMES} -d {} -p tcp -j REDIRECT --to-ports {}",
            crate::infra::reach::STAND_INS,
            crate::infra::reach::NAMES_PORT
        ),
        "COMMIT".to_string(),
    ]);
    rules.join("\n") + "\n"
}

#[derive(Debug, Clone, Copy)]
enum Operation {
    Check,
    InsertAtTop,
}

fn jumps(operation: Operation) -> [Vec<String>; 3] {
    let every_bridge = format!("{BRIDGE_PREFIX}+");
    let at = |table: &str, chain: &str| -> Vec<String> {
        let mut args = vec!["-w", "5", "-t", table];
        match operation {
            Operation::Check => args.extend(["-C", chain]),
            Operation::InsertAtTop => args.extend(["-I", chain, "1"]),
        }
        args.into_iter().map(str::to_string).collect()
    };
    let mut between = at("filter", "FORWARD");
    between.extend(["-j".to_string(), BETWEEN.to_string()]);
    let mut into = at("filter", "INPUT");
    into.extend([
        "-i".to_string(),
        every_bridge.clone(),
        "-j".to_string(),
        INTO_THE_NODE.to_string(),
    ]);
    let mut names = at("nat", "PREROUTING");
    names.extend([
        "-i".to_string(),
        every_bridge,
        "-j".to_string(),
        TO_THE_NAMES.to_string(),
    ]);
    [between, into, names]
}

fn written() -> &'static tokio::sync::Mutex<Option<String>> {
    static WRITTEN: std::sync::OnceLock<tokio::sync::Mutex<Option<String>>> =
        std::sync::OnceLock::new();
    WRITTEN.get_or_init(Default::default)
}

/// Write the rules if the plan says something the kernel was not told.
pub async fn converge(database: &SqliteDatabase, plan: &Plan) {
    pass(database, plan, false).await;
}

/// The same, written again whatever the last pass wrote, because a flush
/// or a restarted firewall service empties the chains and leaves nothing
/// to compare against but the kernel.
pub async fn heal(database: &SqliteDatabase, plan: &Plan) {
    pass(database, plan, true).await;
}

async fn pass(database: &SqliteDatabase, plan: &Plan, checking: bool) {
    if !crate::infra::firewall::armed() {
        return;
    }
    let wanted = render(plan);
    let mut last = written().lock().await;
    let unchanged = last.as_deref() == Some(wanted.as_str());
    if unchanged && !checking {
        return;
    }
    let outcome = tokio::task::spawn_blocking({
        let wanted = wanted.clone();
        move || write(&wanted, unchanged)
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));

    match &outcome {
        Ok(()) => *last = Some(wanted),
        Err(reason) => {
            tracing::error!(%reason, "could not keep the projects apart");
            *last = None;
        }
    }
    record(database, outcome.err().as_deref()).await;
}

fn write(rules: &str, unchanged: bool) -> Result<(), String> {
    let present = jumps(Operation::Check).map(|args| run(&args).is_ok());
    if unchanged && present.iter().all(|present| *present) {
        return Ok(());
    }
    crate::infra::firewall::restore_v4(rules)?;
    for (args, present) in jumps(Operation::InsertAtTop).iter().zip(present) {
        if !present {
            run(args)?;
        }
    }
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let output = std::process::Command::new("iptables")
        .args(args)
        .output()
        .map_err(|error| format!("iptables: {error}"))?;
    match output.status.success() {
        true => Ok(()),
        false => Err(format!(
            "iptables: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

async fn record(database: &SqliteDatabase, reason: Option<&str>) {
    if last_error(database).await.as_deref() == reason {
        return;
    }
    if let Err(error) =
        crate::infra::node::settings::write(database, ERROR, reason.unwrap_or_default()).await
    {
        tracing::warn!(%error, "could not record whether the projects are apart");
    }
}

/// Why the rules could not be written, the last time they were tried.
pub async fn last_error(database: &SqliteDatabase) -> Option<String> {
    crate::infra::node::settings::read(database, ERROR)
        .await
        .ok()
        .flatten()
        .filter(|reason| !reason.trim().is_empty())
}

/// Whether every jump is in the kernel now, for `doctor`.
pub fn in_force() -> Result<(), String> {
    jumps(Operation::Check)
        .iter()
        .try_for_each(|args| run(args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(bridges: &[u8], followed: &[(u8, &str, u16)]) -> Plan {
        Plan {
            bridges: bridges.to_vec(),
            followed: followed
                .iter()
                .map(|(index, address, port)| Followed {
                    index: *index,
                    address: address.parse().expect("an address"),
                    port: *port,
                })
                .collect(),
        }
    }

    fn position(rules: &str, rule: &str) -> usize {
        rules
            .find(rule)
            .unwrap_or_else(|| panic!("{rule:?} is missing from:\n{rules}"))
    }

    #[test]
    fn a_project_reaches_its_own_bridge_and_no_other() {
        let rules = render(&plan(&[1, 2], &[]));
        let own = position(&rules, "-A WABOT-PROJECTS -i wd-1 -o wd-1 -j RETURN");
        position(&rules, "-A WABOT-PROJECTS -i wd-2 -o wd-2 -j RETURN");
        let refused = position(
            &rules,
            "-A WABOT-PROJECTS -i wd-+ -o wd-+ -m comment --comment \"wabot-deploy another project\" -j DROP",
        );
        assert!(own < refused, "{rules}");
        assert!(
            !rules.contains("-i wd-1 -o wd-2"),
            "two projects are never let through by name:\n{rules}"
        );
    }

    #[test]
    fn a_reply_is_never_the_thing_refused() {
        let rules = render(&plan(&[1], &[]));
        for chain in [BETWEEN, INTO_THE_NODE] {
            let reply = position(
                &rules,
                &format!("-A {chain} -m conntrack --ctstate ESTABLISHED,RELATED -j RETURN"),
            );
            let first_drop = rules
                .lines()
                .position(|line| {
                    line.starts_with(&format!("-A {chain} ")) && line.ends_with("DROP")
                })
                .expect("a drop");
            let reply_line = rules[..reply].lines().count();
            assert!(reply_line <= first_drop, "{rules}");
        }
    }

    /// The public door and the back door are one DNAT rule from `portmap`,
    /// matched on any of the node's addresses. Only the original
    /// destination tells them apart: the node's public address is the
    /// door, a bridge's gateway or the overlay address is another
    /// project's copy reached from behind.
    #[test]
    fn a_published_port_is_a_door_only_at_an_address_outside_the_projects() {
        let rules = render(&plan(&[1, 2], &[]));
        let door = position(
            &rules,
            "-A WABOT-PROJECTS -i wd-+ -o wd-+ -m conntrack --ctstate DNAT ! --ctorigdst 10.42.0.0/16 -j RETURN",
        );
        let refused = position(&rules, "-i wd-+ -o wd-+ -m comment");
        assert!(door < refused, "{rules}");
    }

    #[test]
    fn a_container_reaches_the_overlay_only_where_a_standby_follows_its_primary() {
        let rules = render(&plan(&[3], &[(3, "10.42.0.5", 30001)]));
        let follows = position(
            &rules,
            "-A WABOT-PROJECTS -i wd-3 -o wabot0 -d 10.42.0.5/32 -p tcp --dport 30001 -j RETURN",
        );
        let refused = position(
            &rules,
            "-A WABOT-PROJECTS -i wd-+ -o wabot0 -m comment --comment \"wabot-deploy the overlay\" -j DROP",
        );
        assert!(follows < refused, "{rules}");

        let alone = render(&plan(&[3], &[]));
        assert!(!alone.contains("-o wabot0 -d"), "{alone}");
    }

    #[test]
    fn the_overlay_reaches_a_container_only_through_a_port_bound_for_it() {
        let rules = render(&plan(&[1], &[]));
        let bound = position(
            &rules,
            "-A WABOT-PROJECTS -i wabot0 -o wd-+ -m conntrack --ctstate DNAT -j RETURN",
        );
        let refused = position(&rules, "-A WABOT-PROJECTS -i wabot0 -o wd-+ -m comment");
        assert!(bound < refused, "{rules}");
    }

    /// The node's addresses on other bridges and on the overlay are local,
    /// so a packet for them is `INPUT`, never `FORWARD`. A rule set that
    /// only guarded forwarding would let a container reach anything the
    /// node listens on through another project's gateway.
    #[test]
    fn a_container_reaches_the_node_at_its_own_gateway_and_not_at_another_networks() {
        let rules = render(&plan(&[1, 2], &[]));
        let own = position(
            &rules,
            "-A WABOT-PROJECTS-IN -i wd-1 -d 10.42.1.0/24 -j RETURN",
        );
        position(
            &rules,
            "-A WABOT-PROJECTS-IN -i wd-2 -d 10.42.2.0/24 -j RETURN",
        );
        let refused = position(
            &rules,
            "-A WABOT-PROJECTS-IN -i wd-+ -d 10.42.0.0/16 -m comment",
        );
        assert!(own < refused, "{rules}");
    }

    #[test]
    fn a_node_with_no_projects_still_refuses_what_crosses() {
        let rules = render(&Plan::default());
        assert!(rules.starts_with("*filter\n:WABOT-PROJECTS - [0:0]\n:WABOT-PROJECTS-IN - [0:0]\n"));
        assert!(rules.ends_with("COMMIT\n"), "{rules}");
        assert_eq!(rules.matches("-j DROP").count(), 5, "{rules}");
    }

    #[test]
    fn the_rules_only_ever_drop_or_return() {
        let rules = render(&plan(&[1, 2, 254], &[(2, "10.42.0.9", 30010)]));
        let (filter, _) = rules.split_once("*nat").expect("both tables");
        for line in filter.lines().filter(|line| line.starts_with("-A ")) {
            assert!(
                line.ends_with("-j RETURN") || line.ends_with("-j DROP"),
                "{line}"
            );
        }
    }

    /// The only rewrite: a stand-in, which nothing holds, handed to the
    /// node that stands in for it — and only from a bridge, by the jump.
    #[test]
    fn a_stand_in_is_redirected_to_the_node_and_nothing_else_is() {
        let rules = render(&plan(&[1, 2], &[]));
        let (_, nat) = rules.split_once("*nat\n").expect("the nat table");
        let written: Vec<&str> = nat.lines().filter(|line| line.starts_with("-A ")).collect();
        assert_eq!(
            written,
            vec!["-A WABOT-PROJECTS-NAMES -d 10.42.255.0/24 -p tcp -j REDIRECT --to-ports 31100"]
        );
        assert!(nat.starts_with(":WABOT-PROJECTS-NAMES - [0:0]\n"), "{nat}");

        let [_, _, names] = jumps(Operation::InsertAtTop);
        assert_eq!(&names[2..7], &["-t", "nat", "-I", "PREROUTING", "1"]);
        assert!(names.ends_with(&[
            "-i".to_string(),
            "wd-+".to_string(),
            "-j".to_string(),
            TO_THE_NAMES.to_string()
        ]));
    }

    #[test]
    fn both_jumps_go_in_first_and_the_check_asks_for_what_the_insert_writes() {
        let [between, into, _] = jumps(Operation::InsertAtTop);
        assert_eq!(&between[4..7], &["-I", "FORWARD", "1"]);
        assert!(between.ends_with(&["-j".to_string(), BETWEEN.to_string()]));
        assert_eq!(&into[4..7], &["-I", "INPUT", "1"]);
        assert!(into.ends_with(&[
            "-i".to_string(),
            "wd-+".to_string(),
            "-j".to_string(),
            INTO_THE_NODE.to_string()
        ]));

        for (check, insert) in jumps(Operation::Check)
            .iter()
            .zip(jumps(Operation::InsertAtTop))
        {
            assert_eq!(&check[..4], &insert[..4], "the table first");
            assert_eq!(check[5], insert[5], "the same chain");
            assert_eq!(&check[6..], &insert[7..], "the same rule");
        }
    }
}
