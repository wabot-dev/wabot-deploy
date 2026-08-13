//! A network per project, and a namespace per container.
//!
//! ## Why not the host's network
//!
//! Sharing the host namespace is what the runtime did until now, and
//! it makes a service's port a node-wide resource: two projects cannot
//! both run something on 8080, and `nginx:alpine` — which binds 80
//! because that is what its config says — collides with the node's own
//! edge. A container that has to be told which port to bind is a
//! container that has to be modified to be deployed.
//!
//! With a namespace per container and a bridge per project, the port
//! inside is the port the image chose, and the proxy reaches it at the
//! container's own address.
//!
//! ## CNI, and why the plugins rather than our own veth code
//!
//! `bridge` + `host-local` is thirty lines of JSON against a
//! specification that predates this node and outlives it. Writing the
//! netlink by hand would be several hundred lines to reimplement one
//! of them, and every other container tool on the machine already
//! agrees about `/opt/cni/bin`.
//!
//! We invoke the plugins directly rather than through a CNI library:
//! the protocol is "exec this binary with these five environment
//! variables and the config on stdin", and a library for that is more
//! dependency than code.
//!
//! ## One bridge per project, one subnet per bridge
//!
//! Containers in a project can reach each other by address; containers
//! in different projects cannot, because their bridges are separate L2
//! domains and nothing routes between them. That is the isolation the
//! separation is for.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::bootstrap::runtime::{CNI_BIN_DIR, CNI_VERSION};

/// The CNI spec version we speak. Not the plugin version — the two are
/// unrelated, and the plugins support a range.
const CNI_SPEC_VERSION: &str = "1.0.0";

/// Where named network namespaces live. `ip netns` puts them here and
/// so does everything that reads them, including crun.
pub const NETNS_DIR: &str = "/var/run/netns";

/// The interface inside the container. `eth0` because that is what an
/// application, a health check and a person all expect to find.
const IFNAME: &str = "eth0";

/// The address space projects are carved out of.
///
/// `10.42.0.0/16`, one `/24` per project: 254 usable addresses per
/// project and room for 254 projects. Chosen to sit away from the
/// ranges a VPS provider or a Docker installation is likely to be
/// using — `172.17` is Docker's, and `10.0` is what half of every
/// cloud hands out.
const NETWORK_BASE: [u8; 2] = [10, 42];

/// Every address `host-local` may allocate in a project's `/24`.
///
/// `.1` is the bridge itself and `.255` the broadcast; everything
/// between is the plugin's, **including the band the node hands out
/// itself** — see [`RESERVED_HOSTS`] for why they share one range
/// rather than being separated.
const ALLOCATABLE_HOSTS: std::ops::RangeInclusive<u8> = 2..=254;

/// The part the node reserves, for a container whose address is written
/// down somewhere.
///
/// A connection string lives in an application's environment and in
/// somebody's notes, so the address behind it cannot move every time
/// the container is recreated — and left to itself `host-local` hands
/// out the lowest free one, which is stable only while nothing else
/// churns.
///
/// ## One range, and the guarantee comes from the allocator's own books
///
/// Two wrong versions of this shipped in one afternoon, and both were
/// found on a node. They are worth keeping because each is the obvious
/// idea:
///
/// 1. **Bound `host-local` to the low band and keep the high one out of
///    its sight** — the construction the published and overlay port
///    ranges use, where two allocators cannot collide because their
///    ranges do not overlap. It does not work here: `host-local` matches
///    a *requested* address against its configured ranges rather than
///    against the subnet, so asking for one outside them is refused with
///    `requested IP … not in range set`.
/// 2. **Declare both bands as two ranges in one set, low first**, and
///    let the order do the separating. That is legal, and the
///    separation lasts exactly one allocation: the plugin's automatic
///    cursor is `last_reserved_ip`, so handing out a requested `.200`
///    moves it there and the next automatic address is `.201` — inside
///    the reserved band. Observed on a node: a database took `.200`,
///    and the very next container took `.201`.
///
/// So there is **one range covering everything**, and this band is a
/// convention about which end the node picks from rather than a fence.
/// What stops two containers sharing an address is that both go through
/// `host-local`, which keeps one reservation file per address and will
/// not hand out one that exists — a guarantee from its books instead of
/// from arithmetic. The node counts down from the top while the plugin
/// counts up from the bottom, so they meet only after ~250 containers
/// in one project, and when they do the ADD fails by name rather than
/// two containers quietly answering on one address.
pub const RESERVED_HOSTS: std::ops::RangeInclusive<u8> = 200..=254;

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("{0}")]
    Refused(String),
    #[error("{plugin} {command} failed: {detail}")]
    Plugin {
        plugin: &'static str,
        command: &'static str,
        detail: String,
    },
    #[error("{plugin} answered something that is not a CNI result: {detail}")]
    Result {
        plugin: &'static str,
        detail: String,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type NetworkResult<T> = Result<T, NetworkError>;

/// One project's network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectNetwork {
    /// `wd-<index>`. Short on purpose: a Linux interface name is
    /// capped at 15 characters, and a project slug is not.
    pub bridge: String,
    /// The third octet — the project's `/24` inside [`NETWORK_BASE`].
    pub index: u8,
}

impl ProjectNetwork {
    /// The network for a project with this index.
    ///
    /// Indexes start at 1: `10.42.0.0/24` is left alone so that a
    /// misread "no index" never silently means a real network.
    pub fn new(index: u8) -> NetworkResult<Self> {
        if index == 0 {
            return Err(NetworkError::Refused(
                "network index 0 is reserved — projects are numbered from 1".into(),
            ));
        }
        Ok(Self {
            bridge: format!("wd-{index}"),
            index,
        })
    }

    /// The CNI network name. Distinct from the bridge name because
    /// `host-local` keys its address reservations on it, under
    /// `/var/lib/cni/networks/<name>`.
    pub fn name(&self) -> String {
        format!("wabot-deploy-{}", self.index)
    }

    pub fn subnet(&self) -> String {
        let [a, b] = NETWORK_BASE;
        format!("{a}.{b}.{}.0/24", self.index)
    }

    /// An address in the band the node hands out itself.
    ///
    /// Refuses anything outside it, so a caller cannot ask for an
    /// address `host-local` might also give to somebody else — the
    /// separation is only worth having if nothing can cross it.
    pub fn reserved_address(&self, host: u8) -> NetworkResult<Ipv4Addr> {
        if !RESERVED_HOSTS.contains(&host) {
            return Err(NetworkError::Refused(format!(
                "{host} is not in the range this node reserves ({}-{}) — an address \
                 outside it is one the automatic allocator may hand to something else",
                RESERVED_HOSTS.start(),
                RESERVED_HOSTS.end(),
            )));
        }
        let [a, b] = NETWORK_BASE;
        Ok(Ipv4Addr::new(a, b, self.index, host))
    }

    /// The config handed to the `bridge` plugin.
    ///
    /// * `isGateway` — the bridge gets `gateway()`, so containers have
    ///   somewhere to send everything that is not local.
    /// * `ipMasq` — outbound traffic is NATed behind the host's
    ///   address. Without it a container reaches the internet and
    ///   nothing answers, because nothing on the way back knows the
    ///   route to a private `/24` on somebody's VPS.
    /// * `hairpinMode` — a container can reach the bridge address it
    ///   was itself NATed to. Off by default, and its absence shows up
    ///   as a service that can reach every peer except itself.
    ///
    /// ## Asking for one address in particular
    ///
    /// Through `args.cni.ips`, in this config, and **not** through
    /// `CNI_ARGS=IP=`. Both are documented ways to reach `host-local`,
    /// and only one of them survives the chain: the `bridge` plugin
    /// parses `CNI_ARGS` first, into a struct that knows about `MAC=`
    /// and nothing else, and a key it does not recognise is a hard
    /// error — `ARGS: unknown args`. So the environment variable never
    /// reaches the allocator it was meant for.
    ///
    /// Found on a node, by asking the plugin. It would have failed the
    /// first deployment of every database.
    pub fn config(&self, wanted: Option<Ipv4Addr>) -> String {
        // `args` is passed down to the IPAM plugin with the rest of the
        // config, which is how `host-local` sees it at all.
        let args = match wanted {
            Some(address) => {
                format!("\"args\": {{ \"cni\": {{ \"ips\": [\"{address}\"] }} }},\n  ")
            }
            None => String::new(),
        };
        self.config_with(&args)
    }

    fn config_with(&self, args: &str) -> String {
        format!(
            r#"{{
  "cniVersion": "{CNI_SPEC_VERSION}",
  "name": "{name}",
  "type": "bridge",
  "bridge": "{bridge}",
  "isGateway": true,
  "ipMasq": true,
  "hairpinMode": true,
  {args}"ipam": {{
    "type": "host-local",
    "ranges": [[{{ "subnet": "{subnet}", "rangeStart": "{start}", "rangeEnd": "{end}" }}]],
    "routes": [{{ "dst": "0.0.0.0/0" }}]
  }}
}}"#,
            args = args,
            name = self.name(),
            bridge = self.bridge,
            subnet = self.subnet(),
            // One range over everything the plugin may allocate,
            // including the band the node picks from — see
            // `RESERVED_HOSTS`. Sharing the range is what makes
            // `host-local`'s own reservation files the thing that stops
            // a collision.
            //
            // The gateway stays `.1`, outside the range and what
            // `host-local` derives by default — a range bounds what it
            // *allocates*, not what the network contains.
            start = self.host(*ALLOCATABLE_HOSTS.start()),
            end = self.host(*ALLOCATABLE_HOSTS.end()),
        )
    }

    fn host(&self, host: u8) -> String {
        let [a, b] = NETWORK_BASE;
        format!("{a}.{b}.{}.{host}", self.index)
    }

    /// The config handed to the `portmap` plugin.
    ///
    /// `portmap` is a *chained* plugin: it does its work against the
    /// result of the one before it, which arrives as `prevResult`. The
    /// mappings themselves come through `runtimeConfig`, which is how
    /// the CNI spec passes per-container values into a config that is
    /// otherwise per-network.
    ///
    /// On `DEL` the plugin only needs to find and flush its own
    /// chains, so an empty mapping list and an empty previous result
    /// are enough.
    fn portmap_config(&self, mappings: &[PortMapping], prev_result: &str) -> String {
        let ports: Vec<String> = mappings
            .iter()
            .map(|mapping| {
                // `hostIP` omitted rather than sent empty: portmap
                // reads an empty string as an address and refuses it,
                // where an absent key is what means "every interface".
                let host_ip = match &mapping.host_ip {
                    Some(address) => format!(r#", "hostIP": "{address}""#),
                    None => String::new(),
                };
                format!(
                    r#"{{ "hostPort": {}, "containerPort": {}, "protocol": "tcp"{host_ip} }}"#,
                    mapping.host_port, mapping.container_port
                )
            })
            .collect();

        let prev: serde_json::Value = serde_json::from_str(prev_result)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        format!(
            r#"{{
  "cniVersion": "{CNI_SPEC_VERSION}",
  "name": "{name}",
  "type": "portmap",
  "capabilities": {{ "portMappings": true }},
  "runtimeConfig": {{ "portMappings": [{ports}] }},
  "prevResult": {prev}
}}"#,
            name = self.name(),
            ports = ports.join(", "),
        )
    }

    fn loopback_config(&self) -> String {
        format!(
            r#"{{ "cniVersion": "{CNI_SPEC_VERSION}", "name": "{}-lo", "type": "loopback" }}"#,
            self.name()
        )
    }
}

/// The mapping named when tearing a container's port forwarding down.
///
/// `portmap`'s DEL removes the container's *whole* chain — it keys on
/// the container id and the network name, not on what this list says.
/// But it validates the config first, and an empty list makes the call
/// a no-op that leaves every rule in place.
///
/// So the teardown always names one. *Which* one does not matter, and
/// that is not a guess: on a real node, a DEL naming a mapping that
/// had never existed still removed the chain. It has to be this way
/// round, because the case that needs cleaning most is a service that
/// used to publish a port and no longer does — there, the current
/// mappings are empty and the stale DNAT rule is the one still sending
/// the node's port at whatever address the container had last time.
///
/// Port 1 rather than 0: the plugin parses the config before it gets
/// anywhere near the chain, and `0` is not a port. That failure was
/// silent in the way these are — a warning in the journal, a rule left
/// behind, and a node port still answering for a container that had
/// moved.
fn teardown_mapping() -> PortMapping {
    PortMapping {
        host_port: 1,
        container_port: 1,
        host_ip: None,
    }
}

/// The path of a container's network namespace.
pub fn netns_path(container_id: &str) -> PathBuf {
    Path::new(NETNS_DIR).join(container_id)
}

/// One published port: a port on the node, forwarded to one inside
/// the container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    /// Which of the node's addresses to listen on. `None` is all of
    /// them, which is what publishing a port to the world means.
    ///
    /// The overlay needs the other case. A replica another node's edge
    /// has to reach is published on **this node's overlay address and
    /// nowhere else** — the internet has no business with it, and a
    /// port on `0.0.0.0` would hand it over to anybody who guessed the
    /// number.
    pub host_ip: Option<String>,
}

/// Create the namespace, put the container on the project's bridge,
/// and return the address it was given.
///
/// `published` is forwarded by the `portmap` plugin, which chains onto
/// the bridge's result — that is why this is one call rather than two:
/// the chaining needs the bridge's answer, and handing that back for a
/// caller to pass in again would make the CNI protocol somebody else's
/// problem.
///
/// Idempotent by demolition: an existing namespace under this id is
/// torn down first. The reason to attach again is that the previous
/// deployment should stop being the one on the network.
pub async fn attach(
    network: &ProjectNetwork,
    container_id: &str,
    published: &[PortMapping],
    wanted: Option<Ipv4Addr>,
) -> NetworkResult<Ipv4Addr> {
    detach(network, container_id).await;

    create_netns(container_id)?;

    // The order matters: `lo` first, because a container that fails to
    // get an address should still be one we can tear down cleanly.
    plugin(
        "loopback",
        "ADD",
        &network.loopback_config(),
        container_id,
        "lo",
    )
    .await?;

    // The wanted address rides in the *config*, not in `CNI_ARGS` —
    // see `ProjectNetwork::config`. Safe to ask for because the band it
    // comes from is declared as a range of its own, and because the DEL
    // above released whatever this container held last time, which is
    // the reservation it would otherwise collide with.
    let output = plugin(
        "bridge",
        "ADD",
        &network.config(wanted),
        container_id,
        IFNAME,
    )
    .await?;

    let address = first_address(&output).ok_or_else(|| NetworkError::Result {
        plugin: "bridge",
        detail: format!("no address in {output}"),
    })?;

    if !published.is_empty() {
        plugin(
            "portmap",
            "ADD",
            &network.portmap_config(published, &output),
            container_id,
            IFNAME,
        )
        .await?;
    }

    tracing::info!(
        container = container_id,
        %address,
        bridge = %network.bridge,
        published = published.len(),
        "attached"
    );
    Ok(address)
}

/// Take the container off the network and remove its namespace.
///
/// Never fails: this is the cleanup path, and something that already
/// does not exist is the outcome it wanted. Every failure is logged,
/// because a leaked namespace or a leaked address reservation is worth
/// knowing about even when it must not stop a deployment.
pub async fn detach(network: &ProjectNetwork, container_id: &str) {
    // Run even when the namespace is gone.
    //
    // The address reservation is not in the namespace — `host-local`
    // keeps it under `/var/lib/cni/networks/<net>/<ip>`, which
    // survives a reboot while `/run/netns` does not. Skipping the DEL
    // because the namespace had vanished leaked one reservation per
    // container per reboot, and the next deployment failed with
    // "10.42.2.3 has been allocated to <container>, duplicate
    // allocation is not allowed" — about itself.
    //
    // CNI's own contract covers this: DEL must tolerate a missing
    // netns and still release what it allocated.
    {
        // DEL *before* the namespace goes: the plugin removes the host
        // end of the veth, and it needs the namespace to find its way
        // there when there is one.
        for (name, config) in [
            // portmap first, so its rules go before the interface they
            // point at disappears.
            (
                "portmap",
                network.portmap_config(&[teardown_mapping()], "{}"),
            ),
            // No address on the way out: `host-local` releases what
            // this container holds by its id, and naming one it never
            // got would be asking it to free somebody else's.
            ("bridge", network.config(None)),
            ("loopback", network.loopback_config()),
        ] {
            let ifname = if name == "bridge" { IFNAME } else { "lo" };
            // No `IP=` on the way out: `host-local` releases what this
            // container holds by its id, and naming an address it never
            // got would be asking it to free somebody else's.
            if let Err(error) = plugin(name, "DEL", &config, container_id, ifname).await {
                tracing::warn!(container = container_id, %error, "cni DEL");
            }
        }
    }
    if let Err(error) = delete_netns(container_id) {
        tracing::warn!(container = container_id, %error, "removing the namespace");
    }
}

fn create_netns(name: &str) -> NetworkResult<()> {
    std::fs::create_dir_all(NETNS_DIR)?;
    let output = Command::new("ip")
        .args(["netns", "add", name])
        .output()
        .map_err(|error| NetworkError::Refused(format!("could not run `ip`: {error}")))?;

    if !output.status.success() {
        return Err(NetworkError::Refused(format!(
            "could not create the network namespace: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn delete_netns(name: &str) -> NetworkResult<()> {
    if !netns_path(name).exists() {
        return Ok(());
    }
    let output = Command::new("ip")
        .args(["netns", "delete", name])
        .output()?;
    if !output.status.success() {
        return Err(NetworkError::Refused(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

/// Run one CNI plugin.
///
/// The whole protocol: five environment variables, the network config
/// on stdin, the result as JSON on stdout. Errors come back as JSON
/// too, on stdout with a non-zero exit — so stderr alone would often
/// be empty and the reason lost.
async fn plugin(
    name: &'static str,
    command: &'static str,
    config: &str,
    container_id: &str,
    ifname: &str,
) -> NetworkResult<String> {
    use std::io::Write;

    let binary = Path::new(CNI_BIN_DIR).join(name);
    if !binary.exists() {
        return Err(NetworkError::Refused(format!(
            "{} is missing — run `wabot-deploy install` to fetch the CNI plugins {CNI_VERSION}",
            binary.display()
        )));
    }

    let netns = netns_path(container_id);
    let config = config.to_string();
    let container_id = container_id.to_string();
    let ifname = ifname.to_string();

    // Blocking: these are short-lived processes, and giving them their
    // own thread keeps the runtime's executor free rather than parking
    // a worker on a pipe.
    let output = tokio::task::spawn_blocking(move || -> std::io::Result<std::process::Output> {
        // No `CNI_ARGS`. The `bridge` plugin refuses a key it does not
        // recognise, so the one thing a caller might want to say
        // through it — which address to allocate — has to travel in the
        // config instead. See `ProjectNetwork::config`.
        let mut child = Command::new(&binary)
            .env("CNI_COMMAND", command)
            .env("CNI_CONTAINERID", &container_id)
            .env("CNI_NETNS", netns)
            .env("CNI_IFNAME", &ifname)
            .env("CNI_PATH", CNI_BIN_DIR)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        child
            .stdin
            .as_mut()
            .expect("stdin was piped")
            .write_all(config.as_bytes())?;
        child.wait_with_output()
    })
    .await
    .map_err(|error| NetworkError::Plugin {
        plugin: name,
        command,
        detail: error.to_string(),
    })?
    .map_err(|error| NetworkError::Plugin {
        plugin: name,
        command,
        detail: error.to_string(),
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(NetworkError::Plugin {
            plugin: name,
            command,
            detail: plugin_error(&stdout).unwrap_or(if stdout.is_empty() {
                stderr
            } else {
                stdout
            }),
        });
    }
    Ok(stdout)
}

#[derive(Deserialize)]
struct CniError {
    msg: String,
    details: Option<String>,
}

/// The `msg`/`details` a failing plugin writes to stdout.
fn plugin_error(stdout: &str) -> Option<String> {
    let error: CniError = serde_json::from_str(stdout).ok()?;
    Some(match error.details {
        Some(details) if !details.is_empty() => format!("{}: {details}", error.msg),
        _ => error.msg,
    })
}

#[derive(Deserialize)]
struct CniResult {
    #[serde(default)]
    ips: Vec<CniIp>,
}

#[derive(Deserialize)]
struct CniIp {
    address: String,
}

/// The first IPv4 address out of a plugin result.
///
/// `address` is CIDR — `10.42.1.7/24` — because the result describes an
/// interface, not a host.
fn first_address(stdout: &str) -> Option<Ipv4Addr> {
    let result: CniResult = serde_json::from_str(stdout).ok()?;
    result
        .ips
        .iter()
        .find_map(|ip| ip.address.split('/').next()?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_get_their_own_subnet_and_bridge() {
        let one = ProjectNetwork::new(1).expect("valid");
        let two = ProjectNetwork::new(2).expect("valid");

        assert_eq!(one.subnet(), "10.42.1.0/24");
        assert_eq!(two.subnet(), "10.42.2.0/24");
        assert_ne!(one.bridge, two.bridge);
        assert_ne!(one.name(), two.name(), "host-local keys reservations on it");
    }

    /// A Linux interface name is capped at 15 characters, and the cap
    /// is why the bridge is numbered rather than named after the
    /// project.
    #[test]
    fn a_bridge_name_fits_in_an_interface_name() {
        for index in [1u8, 9, 42, 254] {
            let network = ProjectNetwork::new(index).expect("valid");
            assert!(
                network.bridge.len() <= 15,
                "{} is too long for an interface",
                network.bridge
            );
        }
    }

    #[test]
    fn index_zero_is_refused() {
        assert!(ProjectNetwork::new(0).is_err());
    }

    /// The three settings that are the difference between a container
    /// with a network and a container with an address and nothing else.
    #[test]
    fn the_config_carries_what_makes_the_network_work() {
        let config = ProjectNetwork::new(3).expect("valid").config(None);
        for setting in [
            r#""isGateway": true"#,
            r#""ipMasq": true"#,
            r#""hairpinMode": true"#,
            r#""subnet": "10.42.3.0/24""#,
            r#""dst": "0.0.0.0/0""#,
        ] {
            assert!(config.contains(setting), "missing {setting} in {config}");
        }
    }

    /// The band the node picks from has to be **inside** the range
    /// `host-local` allocates, because the plugin matches a requested
    /// address against its configured ranges and not against the
    /// subnet: outside them it answers `requested IP … not in range
    /// set`, which would be every database's first deployment.
    ///
    /// One range over everything, then — see `RESERVED_HOSTS` for the
    /// two tidier separations that were tried and what each did on a
    /// node.
    #[test]
    fn the_node_picks_from_inside_the_range_the_plugin_allocates() {
        let network = ProjectNetwork::new(7).expect("valid");
        let parsed: serde_json::Value =
            serde_json::from_str(&network.config(None)).expect("valid JSON");

        let sets = parsed["ipam"]["ranges"].as_array().expect("ranges");
        assert_eq!(sets.len(), 1, "two sets would be two addresses");
        let ranges = sets[0].as_array().expect("a set of ranges");
        assert_eq!(
            ranges.len(),
            1,
            "one range, so the plugin's books are the guarantee"
        );
        assert_eq!(ranges[0]["rangeStart"], "10.42.7.2");
        assert_eq!(ranges[0]["rangeEnd"], "10.42.7.254");

        // Every address the node may hand out is one the plugin would
        // accept a request for. That is the property the first version
        // broke.
        for host in [*RESERVED_HOSTS.start(), 220, *RESERVED_HOSTS.end()] {
            let reserved = network.reserved_address(host).expect("reserved");
            assert!(
                ALLOCATABLE_HOSTS.contains(&host),
                "{reserved} is outside what the plugin allocates, so a request for it \
                 would be refused"
            );
        }
        assert!(
            network.reserved_address(199).is_err(),
            "the node hands out the top of the range and nothing else"
        );
    }

    #[test]
    fn the_config_is_json_a_plugin_can_read() {
        let config = ProjectNetwork::new(7).expect("valid").config(None);
        let parsed: serde_json::Value = serde_json::from_str(&config).expect("valid JSON");
        assert_eq!(parsed["type"], "bridge");
        assert_eq!(parsed["ipam"]["type"], "host-local");

        let loopback = ProjectNetwork::new(7).expect("valid").loopback_config();
        let parsed: serde_json::Value = serde_json::from_str(&loopback).expect("valid JSON");
        assert_eq!(parsed["type"], "loopback");
    }

    #[test]
    fn the_address_comes_out_of_a_real_plugin_result() {
        // Trimmed from what `bridge` actually writes.
        let output = r#"{
          "cniVersion": "1.0.0",
          "interfaces": [{"name": "wd-1"}, {"name": "eth0", "sandbox": "/var/run/netns/x"}],
          "ips": [{"interface": 2, "address": "10.42.1.7/24", "gateway": "10.42.1.1"}],
          "routes": [{"dst": "0.0.0.0/0"}]
        }"#;
        assert_eq!(first_address(output), Some(Ipv4Addr::new(10, 42, 1, 7)));
    }

    #[test]
    fn a_result_without_an_address_is_not_one() {
        assert_eq!(first_address(r#"{"cniVersion":"1.0.0","ips":[]}"#), None);
        assert_eq!(first_address("not json at all"), None);
    }

    /// A failing plugin writes its reason to *stdout* as JSON. Reading
    /// only stderr leaves an operator with an exit code.
    #[test]
    fn a_plugin_failure_is_read_from_its_json() {
        let stdout = r#"{"cniVersion":"1.0.0","code":100,
            "msg":"failed to allocate for range 0",
            "details":"no IP addresses available in range set: 10.42.1.1-10.42.1.254"}"#;
        let message = plugin_error(stdout).expect("parsed");
        assert!(message.contains("failed to allocate"), "{message}");
        assert!(message.contains("no IP addresses available"), "{message}");
    }

    /// The teardown call has to name a mapping or the plugin does
    /// nothing — and doing nothing leaves a node port pointing at an
    /// address that is no longer there.
    #[test]
    fn the_teardown_config_is_never_an_empty_mapping_list() {
        let config = ProjectNetwork::new(1)
            .expect("valid")
            .portmap_config(&[teardown_mapping()], "{}");

        let parsed: serde_json::Value = serde_json::from_str(&config).expect("valid JSON");
        assert_eq!(parsed["type"], "portmap");
        assert_eq!(
            parsed["runtimeConfig"]["portMappings"]
                .as_array()
                .map(Vec::len),
            Some(1),
            "an empty list makes DEL a no-op"
        );
    }

    /// A replica another node's edge has to reach is published on this
    /// node's **overlay address and nowhere else**. On `0.0.0.0` it
    /// would be handed to anybody on the internet who guessed the
    /// number, which is not what placing a replica somewhere asked for.
    #[test]
    fn an_overlay_port_is_bound_to_one_address() {
        let config = ProjectNetwork::new(1).expect("valid").portmap_config(
            &[PortMapping {
                host_port: 20002,
                container_port: 80,
                host_ip: Some("10.42.0.3".into()),
            }],
            r#"{"cniVersion":"1.0.0","ips":[{"address":"10.42.1.7/24"}]}"#,
        );

        let parsed: serde_json::Value = serde_json::from_str(&config).expect("valid JSON");
        let mapping = &parsed["runtimeConfig"]["portMappings"][0];
        assert_eq!(mapping["hostPort"], 20002);
        assert_eq!(mapping["hostIP"], "10.42.0.3");
    }

    #[test]
    fn a_published_port_becomes_a_mapping_the_plugin_understands() {
        let config = ProjectNetwork::new(1).expect("valid").portmap_config(
            &[PortMapping {
                host_port: 20001,
                container_port: 80,
                host_ip: None,
            }],
            r#"{"cniVersion":"1.0.0","ips":[{"address":"10.42.1.7/24"}]}"#,
        );

        let parsed: serde_json::Value = serde_json::from_str(&config).expect("valid JSON");
        let mapping = &parsed["runtimeConfig"]["portMappings"][0];
        assert_eq!(mapping["hostPort"], 20001);
        assert_eq!(mapping["containerPort"], 80);
        assert_eq!(mapping["protocol"], "tcp");
        assert!(
            mapping.get("hostIP").is_none(),
            "an absent key is what means every interface; portmap reads an \
             empty string as an address and refuses it"
        );
        // Chained: without the previous result the plugin does not know
        // which address to forward to.
        assert_eq!(parsed["prevResult"]["ips"][0]["address"], "10.42.1.7/24");
        assert_eq!(parsed["capabilities"]["portMappings"], true);
    }

    /// The reservation lives outside the namespace, so the teardown
    /// cannot be conditional on the namespace existing. It was, and a
    /// reboot — which empties `/run/netns` and keeps
    /// `/var/lib/cni/networks` — leaked one address per container.
    /// The next deployment then failed with "duplicate allocation"
    /// about a container that was itself.
    #[test]
    fn the_teardown_is_not_conditional_on_a_namespace() {
        let source = include_str!("network.rs");
        let detach = source
            .split_once("pub async fn detach(")
            .expect("detach exists")
            .1;
        let body = detach
            .split_once("\nfn create_netns")
            .map(|(body, _)| body)
            .unwrap_or(detach);

        assert!(
            !body.contains("if netns_path(container_id).exists()"),
            "the CNI DEL is gated on the namespace again — a reboot will leak every address"
        );
    }

    #[test]
    fn the_namespace_path_is_the_one_ip_netns_uses() {
        assert_eq!(
            netns_path("my-api--web"),
            Path::new("/var/run/netns/my-api--web")
        );
    }
}
