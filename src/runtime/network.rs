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
    pub fn config(&self) -> String {
        format!(
            r#"{{
  "cniVersion": "{CNI_SPEC_VERSION}",
  "name": "{name}",
  "type": "bridge",
  "bridge": "{bridge}",
  "isGateway": true,
  "ipMasq": true,
  "hairpinMode": true,
  "ipam": {{
    "type": "host-local",
    "ranges": [[{{ "subnet": "{subnet}" }}]],
    "routes": [{{ "dst": "0.0.0.0/0" }}]
  }}
}}"#,
            name = self.name(),
            bridge = self.bridge,
            subnet = self.subnet(),
        )
    }

    fn loopback_config(&self) -> String {
        format!(
            r#"{{ "cniVersion": "{CNI_SPEC_VERSION}", "name": "{}-lo", "type": "loopback" }}"#,
            self.name()
        )
    }
}

/// The path of a container's network namespace.
pub fn netns_path(container_id: &str) -> PathBuf {
    Path::new(NETNS_DIR).join(container_id)
}

/// Create the namespace, put the container on the project's bridge,
/// and return the address it was given.
///
/// Idempotent by demolition: an existing namespace under this id is
/// torn down first. The reason to attach again is that the previous
/// deployment should stop being the one on the network.
pub async fn attach(network: &ProjectNetwork, container_id: &str) -> NetworkResult<Ipv4Addr> {
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

    let output = plugin("bridge", "ADD", &network.config(), container_id, IFNAME).await?;

    let address = first_address(&output).ok_or_else(|| NetworkError::Result {
        plugin: "bridge",
        detail: format!("no address in {output}"),
    })?;

    tracing::info!(container = container_id, %address, bridge = %network.bridge, "attached");
    Ok(address)
}

/// Take the container off the network and remove its namespace.
///
/// Never fails: this is the cleanup path, and something that already
/// does not exist is the outcome it wanted. Every failure is logged,
/// because a leaked namespace or a leaked address reservation is worth
/// knowing about even when it must not stop a deployment.
pub async fn detach(network: &ProjectNetwork, container_id: &str) {
    if netns_path(container_id).exists() {
        // DEL *before* the namespace goes: the plugin releases the
        // address reservation and removes the host end of the veth,
        // and it needs the namespace to find its way there.
        for (name, config) in [
            ("bridge", network.config()),
            ("loopback", network.loopback_config()),
        ] {
            let ifname = if name == "bridge" { IFNAME } else { "lo" };
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
        let config = ProjectNetwork::new(3).expect("valid").config();
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

    #[test]
    fn the_config_is_json_a_plugin_can_read() {
        let config = ProjectNetwork::new(7).expect("valid").config();
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

    #[test]
    fn the_namespace_path_is_the_one_ip_netns_uses() {
        assert_eq!(
            netns_path("my-api--web"),
            Path::new("/var/run/netns/my-api--web")
        );
    }
}
