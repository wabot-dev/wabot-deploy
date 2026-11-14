//! The allowlists the edge cannot see: SSH, and every port a service
//! publishes as raw TCP.
//!
//! ## One chain, in `mangle PREROUTING`
//!
//! A published port is two different things below this process: a DNAT
//! the CNI `portmap` plugin wrote, which sends the connection on to a
//! container through `FORWARD`, and a listener this process holds for a
//! database whose primary runs elsewhere, which is `INPUT`. Before either,
//! in `PREROUTING`, both are a packet for one of this machine's addresses
//! on that port. So the chain sits there, ahead of `nat`, and one rule set
//! guards both without knowing which a port is.
//!
//! `mangle` cannot `REJECT`, so a refused connection is dropped and the
//! client waits out its timeout. That is what a cloud provider's security
//! group does, and the cost of the single place.
//!
//! ## New connections only
//!
//! The jump matches `--ctstate NEW`. A connection already open when a list
//! changes stays open, as it does behind a security group — and an
//! operator who closes SSH from inside an SSH session keeps the session.
//!
//! ## Always let in
//!
//! The machine itself, and the overlay: the same two the edge never
//! refuses. The overlay's peers are the network's nodes, authenticated by
//! WireGuard, and a forward from the owner of a database reaches its
//! primary on another node that way.
//!
//! ## Written whole
//!
//! `iptables-restore --noflush` with the chain declared, which empties it
//! and writes it again in one transaction, so there is no moment with half
//! the rules. Nothing outside the chain is touched but the one jump.
//!
//! ## Only where it is armed
//!
//! The rules belong to the node, so only `serve` and the recovery verb arm
//! this. A test runs as root inside the VM, where an unarmed pass is the
//! difference between testing the node and rewriting its firewall.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::domain::errors::PlatformResult;
use ipnet::IpNet;
use wabot::sqlite::SqliteDatabase;

use crate::domain::values::allowlist::Door;

pub const CHAIN: &str = "WABOT-ALLOW";

const ERROR: &str = "firewall.last_error";

static ARMED: AtomicBool = AtomicBool::new(false);

/// Let this process write the machine's rules.
pub fn arm() {
    ARMED.store(true, Ordering::SeqCst);
}

fn armed() -> bool {
    ARMED.load(Ordering::SeqCst)
}

/// One port, and the list that guards it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    pub port: u16,
    pub door: Door,
    pub networks: Vec<IpNet>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    V4,
    V6,
}

impl Family {
    fn holds(self, network: &IpNet) -> bool {
        matches!(
            (self, network),
            (Family::V4, IpNet::V4(_)) | (Family::V6, IpNet::V6(_))
        )
    }

    fn restore(self) -> &'static str {
        match self {
            Family::V4 => "iptables-restore",
            Family::V6 => "ip6tables-restore",
        }
    }

    fn tables(self) -> &'static str {
        match self {
            Family::V4 => "iptables",
            Family::V6 => "ip6tables",
        }
    }
}

/// Every port the kernel guards, with its list, read off the rows.
///
/// A port a service publishes is its own here: a service that arrived on
/// an errand publishes nothing, so every published port is one this node
/// administers, and so is its list.
pub async fn gates(database: &SqliteDatabase) -> PlatformResult<Vec<Gate>> {
    let lists = crate::infra::repositories::allowlists::all(database).await?;
    let list_of = |door: &Door| lists.get(door).cloned().unwrap_or_default();

    let mut gates: Vec<Gate> = ssh_ports()
        .await
        .into_iter()
        .map(|port| Gate {
            port,
            door: Door::Ssh,
            networks: list_of(&Door::Ssh),
        })
        .collect();
    for port in crate::infra::repositories::ports::all(database).await? {
        let Some(host_port) = port.host_port else {
            continue;
        };
        let door = Door::Service(port.service_id.clone());
        gates.push(Gate {
            port: host_port,
            networks: list_of(&door),
            door,
        });
    }
    gates.sort_by_key(|gate| gate.port);
    gates.dedup_by_key(|gate| gate.port);
    Ok(gates)
}

/// The input `iptables-restore --noflush` is given for one family.
pub fn render(gates: &[Gate], family: Family) -> String {
    let mut rules = vec![
        "*mangle".to_string(),
        format!(":{CHAIN} - [0:0]"),
        format!("-A {CHAIN} -i lo -j RETURN"),
        format!(
            "-A {CHAIN} -i {} -j RETURN",
            crate::infra::network::tunnel::INTERFACE
        ),
        format!("-A {CHAIN} -m addrtype --src-type LOCAL -j RETURN"),
    ];
    for gate in gates {
        let networks: Vec<&IpNet> = gate
            .networks
            .iter()
            .filter(|network| family.holds(network))
            .collect();
        if networks.iter().any(|network| network.prefix_len() == 0) {
            continue;
        }
        for network in networks {
            rules.push(format!(
                "-A {CHAIN} -p tcp --dport {} -s {network} -j RETURN",
                gate.port
            ));
        }
        rules.push(format!(
            "-A {CHAIN} -p tcp --dport {} -m comment --comment \"{}\" -j DROP",
            gate.port,
            described(&gate.door)
        ));
    }
    rules.push("COMMIT".to_string());
    rules.join("\n") + "\n"
}

fn described(door: &Door) -> String {
    match door.columns() {
        (kind, "") => format!("wabot-deploy {kind}"),
        (kind, subject) => format!("wabot-deploy {kind} {subject}"),
    }
}

/// The jump into the chain, as `iptables` takes it for `-C` and for `-I`.
///
/// The table before the operation: `iptables -I -t mangle` reads `-t` as
/// the chain's name, which is what the first version of this did on the VM
/// node — the chain was written and nothing jumped to it.
fn jump(operation: Operation) -> Vec<&'static str> {
    let mut args = vec!["-w", "5", "-t", "mangle"];
    match operation {
        Operation::Check => args.extend(["-C", "PREROUTING"]),
        Operation::InsertAtTop => args.extend(["-I", "PREROUTING", "1"]),
    }
    args.extend([
        "-m",
        "addrtype",
        "--dst-type",
        "LOCAL",
        "-p",
        "tcp",
        "-m",
        "conntrack",
        "--ctstate",
        "NEW",
        "-j",
        CHAIN,
    ]);
    args
}

#[derive(Debug, Clone, Copy)]
enum Operation {
    Check,
    InsertAtTop,
}

/// What was written last, so a pass that changes nothing writes nothing.
fn written() -> &'static tokio::sync::Mutex<Option<String>> {
    static WRITTEN: std::sync::OnceLock<tokio::sync::Mutex<Option<String>>> =
        std::sync::OnceLock::new();
    WRITTEN.get_or_init(Default::default)
}

/// Write the rules if the rows say something the kernel was not told.
pub async fn converge(database: &SqliteDatabase) {
    pass(database, false).await;
}

/// The same, written again whatever the last pass wrote: a flush, another
/// tool or a restarted firewall service can empty the chain and leave the
/// jump, and asking only about the jump took that for in force. One
/// transaction every ten seconds is the price of not having to tell.
pub async fn heal(database: &SqliteDatabase) {
    pass(database, true).await;
}

async fn pass(database: &SqliteDatabase, checking: bool) {
    if !armed() {
        return;
    }
    let gates = match gates(database).await {
        Ok(gates) => gates,
        Err(error) => {
            tracing::warn!(%error, "could not read what the firewall guards");
            return;
        }
    };
    let wanted = [Family::V4, Family::V6]
        .map(|family| render(&gates, family))
        .join("");

    let mut last = written().lock().await;
    let unchanged = last.as_deref() == Some(wanted.as_str());
    if unchanged && !checking {
        return;
    }
    let outcome = tokio::task::spawn_blocking({
        let gates = gates.clone();
        move || write(&gates, unchanged && !checking)
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));

    match &outcome {
        Ok(()) => *last = Some(wanted),
        Err(reason) => {
            tracing::error!(%reason, "could not write the firewall's rules");
            *last = None;
        }
    }
    record(database, outcome.err().as_deref()).await;
}

fn write(gates: &[Gate], unchanged: bool) -> Result<(), String> {
    for family in [Family::V4, Family::V6] {
        if family == Family::V6 && !std::path::Path::new("/proc/net/if_inet6").exists() {
            continue;
        }
        let present = run(family.tables(), &jump(Operation::Check)).is_ok();
        if unchanged && present {
            continue;
        }
        restore(family, &render(gates, family))?;
        if !present {
            run(family.tables(), &jump(Operation::InsertAtTop))?;
        }
    }
    Ok(())
}

fn restore(family: Family, rules: &str) -> Result<(), String> {
    use std::io::Write;
    let mut child = std::process::Command::new(family.restore())
        .args(["-w", "5", "--noflush"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| format!("{}: {error}", family.restore()))?;
    child
        .stdin
        .take()
        .ok_or_else(|| format!("{}: no input", family.restore()))?
        .write_all(rules.as_bytes())
        .map_err(|error| format!("{}: {error}", family.restore()))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("{}: {error}", family.restore()))?;
    match output.status.success() {
        true => Ok(()),
        false => Err(format!(
            "{}: {}",
            family.restore(),
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{program}: {error}"))?;
    match output.status.success() {
        true => Ok(()),
        false => Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

async fn record(database: &SqliteDatabase, reason: Option<&str>) {
    let stored = last_error(database).await;
    if stored.as_deref() == reason {
        return;
    }
    if let Err(error) =
        crate::infra::node::settings::write(database, ERROR, reason.unwrap_or_default()).await
    {
        tracing::warn!(%error, "could not record the firewall's state");
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

/// Whether the jump into the chain is in the kernel now, for `doctor`.
pub fn in_force() -> Result<(), String> {
    run(Family::V4.tables(), &jump(Operation::Check))
}

/// When `sshd` was last asked, and what it said.
type Asked = (std::time::Instant, Vec<u16>);

/// The ports the machine's SSH server listens on, as it reports them.
///
/// Asked of `sshd -T` rather than read from the file: `Include`, `Match`
/// and the defaults are its to resolve. 22 when it cannot be asked — a
/// rule on a port nothing listens on costs nothing, and no rule on the one
/// something does is the mistake.
pub async fn ssh_ports() -> Vec<u16> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<Asked>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((asked, ports)) = cache.lock().ok().and_then(|held| held.clone()) {
        if asked.elapsed() < std::time::Duration::from_secs(60) {
            return ports;
        }
    }
    let ports = tokio::task::spawn_blocking(|| {
        ["sshd", "/usr/sbin/sshd"]
            .into_iter()
            .find_map(|program| {
                let output = std::process::Command::new(program)
                    .arg("-T")
                    .output()
                    .ok()?;
                output
                    .status
                    .success()
                    .then(|| ports_in(&String::from_utf8_lossy(&output.stdout)))
                    .filter(|ports| !ports.is_empty())
            })
            .unwrap_or_else(|| vec![22])
    })
    .await
    .unwrap_or_else(|_| vec![22]);
    if let Ok(mut held) = cache.lock() {
        *held = Some((std::time::Instant::now(), ports.clone()));
    }
    ports
}

fn ports_in(effective: &str) -> Vec<u16> {
    let mut ports: Vec<u16> = effective
        .lines()
        .filter_map(|line| line.strip_prefix("port "))
        .filter_map(|port| port.trim().parse().ok())
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(port: u16, door: Door, networks: &[&str]) -> Gate {
        Gate {
            port,
            door,
            networks: networks
                .iter()
                .map(|network| network.parse().expect("a range"))
                .collect(),
        }
    }

    #[test]
    fn the_machine_and_the_overlay_are_let_in_before_any_port_is_asked_about() {
        let rules = render(&[gate(22, Door::Ssh, &[])], Family::V4);
        let first_port = rules.find("--dport").expect("a port rule");
        for exemption in [
            "-i lo -j RETURN",
            "-i wabot0 -j RETURN",
            "--src-type LOCAL -j RETURN",
        ] {
            let at = rules.find(exemption).expect(exemption);
            assert!(at < first_port, "{exemption} comes first:\n{rules}");
        }
    }

    #[test]
    fn an_empty_list_drops_every_new_connection_to_its_port() {
        let rules = render(
            &[gate(5432, Door::Service("svc-1".into()), &[])],
            Family::V4,
        );
        assert!(
            rules.contains(
                "-A WABOT-ALLOW -p tcp --dport 5432 -m comment --comment \"wabot-deploy service svc-1\" -j DROP"
            ),
            "{rules}"
        );
        assert!(!rules.contains("--dport 5432 -s"), "{rules}");
    }

    #[test]
    fn a_listed_range_returns_ahead_of_the_drop() {
        let rules = render(
            &[gate(22, Door::Ssh, &["203.0.113.0/24", "2001:db8::/32"])],
            Family::V4,
        );
        let allowed = rules
            .find("--dport 22 -s 203.0.113.0/24 -j RETURN")
            .expect("allowed");
        let dropped = rules.find("--dport 22 -m comment").expect("dropped");
        assert!(allowed < dropped, "{rules}");
        assert!(
            !rules.contains("2001:db8"),
            "a v6 range is not a v4 rule:\n{rules}"
        );

        let six = render(
            &[gate(22, Door::Ssh, &["203.0.113.0/24", "2001:db8::/32"])],
            Family::V6,
        );
        assert!(
            six.contains("--dport 22 -s 2001:db8::/32 -j RETURN"),
            "{six}"
        );
        assert!(!six.contains("203.0.113"), "{six}");
    }

    #[test]
    fn everyone_in_a_family_writes_nothing_for_that_family() {
        let open = gate(22, Door::Ssh, &["0.0.0.0/0", "::/0"]);
        for family in [Family::V4, Family::V6] {
            assert!(!render(std::slice::from_ref(&open), family).contains("--dport"));
        }
        let only_four = gate(22, Door::Ssh, &["0.0.0.0/0"]);
        assert!(!render(std::slice::from_ref(&only_four), Family::V4).contains("--dport"));
        assert!(
            render(&[only_four], Family::V6).contains("--dport 22 -m comment"),
            "0.0.0.0/0 is not everyone over IPv6"
        );
    }

    #[test]
    fn the_rules_are_one_transaction_that_empties_the_chain_first() {
        let rules = render(&[], Family::V4);
        assert!(
            rules.starts_with("*mangle\n:WABOT-ALLOW - [0:0]\n"),
            "{rules}"
        );
        assert!(rules.ends_with("COMMIT\n"), "{rules}");
    }

    #[test]
    fn the_ssh_ports_are_the_ones_sshd_reports() {
        assert_eq!(
            ports_in("port 2222\nlistenaddress 0.0.0.0:2222\nport 22\npermitrootlogin yes\n"),
            vec![22, 2222]
        );
        assert!(ports_in("usepam yes\n").is_empty());
    }

    /// The first version put `-I` ahead of `-t mangle`, which iptables
    /// reads as a chain called `-t`: the rules were written and nothing
    /// jumped to them. Found on the VM node.
    #[test]
    fn the_table_comes_before_the_operation_and_the_jump_goes_in_first() {
        let insert = jump(Operation::InsertAtTop);
        let table = insert.iter().position(|arg| *arg == "-t").expect("a table");
        let operation = insert
            .iter()
            .position(|arg| *arg == "-I")
            .expect("an insert");
        assert!(table < operation, "{insert:?}");
        assert_eq!(
            &insert[operation..operation + 3],
            &["-I", "PREROUTING", "1"]
        );
        assert!(insert.ends_with(&["-j", CHAIN]));

        let check = jump(Operation::Check);
        let operation = check.iter().position(|arg| *arg == "-C").expect("a check");
        assert!(table < operation, "{check:?}");
        assert_eq!(
            &check[operation + 2..],
            &insert[insert.iter().position(|arg| *arg == "1").expect("") + 1..],
            "the check asks for exactly the rule the insert writes"
        );
    }

    #[tokio::test]
    async fn every_published_port_is_guarded_by_its_services_list_and_ssh_by_its_own() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &database,
            &project.id,
            "db",
            "docker.io/library/postgres:17",
            &[],
        )
        .await
        .expect("service");
        let published = crate::infra::repositories::ports::create(
            &database,
            &service.id,
            5432,
            false,
            None,
            &[],
        )
        .await
        .expect("port");
        crate::infra::repositories::ports::create(&database, &service.id, 9000, false, None, &[])
            .await
            .expect("an unpublished port");
        let host_port =
            crate::infra::repositories::ports::set_published(&database, &published.id, true)
                .await
                .expect("published");
        crate::infra::repositories::allowlists::set(
            &database,
            &Door::Service(service.id.clone()),
            &crate::domain::values::allowlist::Allowlist::parse("203.0.113.5").expect("parsed"),
        )
        .await
        .expect("list");

        let found = gates(&database).await.expect("gates");
        let guarded: Vec<(u16, &[IpNet])> = found
            .iter()
            .filter(|gate| gate.door == Door::Service(service.id.clone()))
            .map(|gate| (gate.port, gate.networks.as_slice()))
            .collect();
        assert_eq!(
            guarded,
            vec![(
                host_port.expect("a host port"),
                &["203.0.113.5/32".parse::<IpNet>().expect("a range")][..]
            )],
            "the published port and not the other: {found:?}"
        );
        let ssh = found
            .iter()
            .find(|gate| gate.door == Door::Ssh)
            .expect("ssh is guarded");
        assert!(
            crate::domain::values::allowlist::is_everyone(&ssh.networks),
            "opened by the migration: {found:?}"
        );
    }
}
