//! The overlay: one WireGuard interface, carried by the kernel.
//!
//! ## Why the kernel, against what was planned
//!
//! `docs/network.md` argued for `boringtun` on the grounds that
//! requiring a kernel module reopens the class of failure Alpine keeps
//! surfacing. The spike said otherwise, on both nodes: the `wireguard`
//! module is in both kernels and `ip link add type wireguard` creates
//! the interface with no `wireguard-tools` present — while on Alpine
//! `/dev/net/tun`, which userspace WireGuard needs, does **not exist**
//! until `tun` is loaded. Userspace does not avoid a module; it swaps
//! one module for another and adds the entire data path.
//!
//! ## Convergent, like everything else here
//!
//! This asks what the `node` table says the overlay should be and makes
//! the interface match. It does not ask whether it has run before —
//! called at every start, and again whenever a node joins or is
//! enrolled, because both change the answer.
//!
//! ## Who dials whom
//!
//! A private node sets an endpoint for its authority and a keepalive; a
//! public node sets neither for a private peer. That asymmetry is the
//! whole reason this works behind NAT: the private node opens the UDP
//! session outbound, the keepalive holds the translation open, and the
//! public node learns where to answer from the handshake that arrives.
//! Configuring an endpoint on the public side would be guessing an
//! address the NAT owns.

use std::net::SocketAddr;

use defguard_wireguard_rs::{key::Key, net::IpAddrMask, peer::Peer};
#[cfg(target_os = "linux")]
use defguard_wireguard_rs::{InterfaceConfiguration, Kernel, WGApi, WireguardInterfaceApi};
use wabot::sqlite::SqliteDatabase;

use super::{keys, NetworkError, NetworkResult, Node};
use crate::config::Config;

/// What the interface is called.
///
/// Not `wg0`: that is what everything else on a machine also picks, and
/// a node that quietly took over somebody's existing tunnel would be
/// the worst possible way to find that out.
pub const INTERFACE: &str = "wabot0";

/// The port WireGuard listens on, when the config does not say.
pub const DEFAULT_PORT: u16 = 51820;

/// How often a node behind NAT reminds the translation it is still
/// there. WireGuard's own recommendation, and comfortably under the
/// shortest UDP timeout worth worrying about.
const KEEPALIVE: u16 = 25;

/// Bring the overlay to what the `node` table says it should be.
///
/// `Ok(None)` when this node is not on an overlay at all — no address
/// or no key — which is every node that has neither joined anything nor
/// enrolled anybody. Doing nothing is the whole of the right behaviour
/// there: an interface for a network of one is a network interface
/// nothing will ever send a packet to.
pub async fn ensure(database: &SqliteDatabase, config: &Config) -> NetworkResult<Option<Overlay>> {
    let Some(me) = super::me(database).await? else {
        return Ok(None);
    };
    let (Some(address), Some(private)) = (me.overlay_ip.clone(), keys::private_key(database).await)
    else {
        return Ok(None);
    };

    let peers = peers(database, config, &me).await?;
    let overlay = Overlay {
        // With the mask, because that is what the interface carries and
        // what somebody comparing it against `ip addr` will see.
        address: interface_address(&address),
        port: config.overlay.port,
        peers: peers.len(),
    };

    // Everything above is a database read and could not fail on the
    // machine. Everything below touches the kernel, and on a node where
    // that is refused the rest of the node must keep working — the
    // console is where somebody goes to find out why.
    if let Err(error) = apply(&address, &private, config.overlay.port, peers) {
        return Err(NetworkError::Refused(error));
    }
    Ok(Some(overlay))
}

/// What came up, for a caller that has to report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    pub address: String,
    pub port: u16,
    pub peers: usize,
}

/// Every other node this one shares an overlay with.
///
/// A node with no key or no address is one that has not finished
/// joining. It is skipped rather than refused: the row is legitimate,
/// it is simply not something WireGuard can be told about yet.
async fn peers(database: &SqliteDatabase, config: &Config, me: &Node) -> NetworkResult<Vec<Peer>> {
    let mut peers = Vec::new();

    for node in super::all(database).await? {
        if node.id == me.id {
            continue;
        }
        let (Some(public_key), Some(address)) = (&node.public_key, &node.overlay_ip) else {
            continue;
        };
        let (Ok(key), Ok(allowed)) = (
            Key::try_from(public_key.as_str()),
            format!("{address}/32").parse::<IpAddrMask>(),
        ) else {
            tracing::warn!(node = %node.id, "skipping a peer whose key or address will not parse");
            continue;
        };

        let mut peer = Peer::new(key);
        // One address per peer, not the whole subnet. AllowedIPs is
        // WireGuard's authorisation as well as its routing table: a
        // peer allowed the whole `/16` could send a packet claiming to
        // be any node on the overlay, and the kernel would accept it.
        peer.allowed_ips = vec![allowed];

        // Only towards a node this one can dial. See the module docs:
        // the other direction is learned from the handshake.
        if let Some(endpoint) = overlay_endpoint(&node, config).await {
            peer.endpoint = Some(endpoint);
            peer.persistent_keepalive_interval = Some(KEEPALIVE);
        }
        peers.push(peer);
    }

    Ok(peers)
}

/// Where to send this peer's packets, if anywhere.
///
/// The host comes from the control-plane endpoint and the port from the
/// config, because they are different ports on the same machine — the
/// console answers TCP 443, WireGuard answers UDP 51820. Resolved here
/// rather than stored, so a node whose address changes is reached at
/// the new one on the next start without anybody editing a row.
async fn overlay_endpoint(node: &Node, config: &Config) -> Option<SocketAddr> {
    let endpoint = node.endpoint.as_deref()?;
    // `host:port`, and the port is the control plane's — dropped.
    let host = endpoint.rsplit_once(':').map(|(host, _)| host)?;

    match tokio::net::lookup_host((host, config.overlay.port)).await {
        Ok(mut addresses) => addresses.next(),
        Err(error) => {
            // Not fatal: the peer is configured without an endpoint and
            // the tunnel still comes up if *it* dials us. A node whose
            // DNS is briefly broken should not take the overlay down.
            tracing::warn!(%error, node = %node.id, "could not resolve a peer's address");
            None
        }
    }
}

/// The address the interface carries.
///
/// The whole subnet's mask on this node's own address, not a `/32`:
/// that is what puts a route to every other overlay address through
/// this interface. With a `/32` the kernel would have an address and no
/// way to reach anybody using it.
fn interface_address(address: &str) -> String {
    format!("{address}/{}", super::overlay::PREFIX_LENGTH)
}

/// The part that talks to the kernel.
///
/// Separated for two reasons. Everything above it is async and this is
/// not — the netlink calls block, and they are short enough that a
/// `spawn_blocking` around each would cost more than it saves. And this
/// is the half that only exists on Linux, so everything worth testing
/// on a developer's machine is above the line rather than below it.
#[cfg(target_os = "linux")]
fn apply(address: &str, private: &str, port: u16, peers: Vec<Peer>) -> Result<(), String> {
    let mut api = WGApi::<Kernel>::new(INTERFACE)
        .map_err(|error| format!("could not open netlink: {error}"))?;

    // Creating an interface that is already there is the normal case —
    // this runs at every start — so an error here is only interesting
    // if the interface did not end up existing, which the configure
    // below reports.
    if let Err(error) = api.create_interface() {
        tracing::debug!(%error, "the overlay interface was already there");
    }

    let addresses = vec![interface_address(address)
        .parse::<IpAddrMask>()
        .map_err(|error| format!("{address} is not an address: {error}"))?];

    api.configure_interface(&InterfaceConfiguration {
        name: INTERFACE.to_string(),
        prvkey: private.to_string(),
        addresses,
        port,
        peers,
        mtu: None,
        fwmark: None,
    })
    .map_err(|error| format!("could not configure {INTERFACE}: {error}"))
}

/// The product is a Linux daemon. These exist so the crate still
/// builds — and its tests still run — on whatever machine it is being
/// written on, which is the same reason `install::restrict` has a
/// second body. A node that reaches these has no kernel to ask.
#[cfg(not(target_os = "linux"))]
fn apply(_address: &str, _private: &str, _port: u16, _peers: Vec<Peer>) -> Result<(), String> {
    Err("the overlay needs a Linux kernel".into())
}

#[cfg(not(target_os = "linux"))]
pub fn observed() -> Result<Vec<Handshake>, String> {
    Err("the overlay needs a Linux kernel".into())
}

/// What the interface is actually doing, read back from the kernel.
///
/// Read rather than remembered. The point of reporting an overlay is to
/// say whether packets are moving, and the only thing that knows is the
/// kernel — a struct this process filled in at startup would report
/// what it asked for, which is the question nobody has.
#[cfg(target_os = "linux")]
pub fn observed() -> Result<Vec<Handshake>, String> {
    let api = WGApi::<Kernel>::new(INTERFACE)
        .map_err(|error| format!("could not open netlink: {error}"))?;
    let host = api
        .read_interface_data()
        .map_err(|error| format!("could not read {INTERFACE}: {error}"))?;

    let mut seen: Vec<Handshake> = host
        .peers
        .into_values()
        .map(|peer| Handshake {
            public_key: peer.public_key.to_string(),
            endpoint: peer.endpoint.map(|address| address.to_string()),
            last_handshake: peer
                .last_handshake
                .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|since| since.as_secs())
                .filter(|seconds| *seconds > 0),
            tx_bytes: peer.tx_bytes,
            rx_bytes: peer.rx_bytes,
        })
        .collect();
    seen.sort_by(|a, b| a.public_key.cmp(&b.public_key));
    Ok(seen)
}

/// One peer, as the kernel sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub public_key: String,
    /// Where packets are going, which for a peer this node did not dial
    /// is where its handshake came *from*.
    pub endpoint: Option<String>,
    /// Unix seconds. `None` means no handshake has ever completed —
    /// the peer is configured and has never been heard from, which is
    /// the failure an operator most needs to tell apart from working.
    pub last_handshake: Option<u64>,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
}

impl Handshake {
    /// Whether this peer has ever answered.
    pub fn live(&self) -> bool {
        self.last_handshake.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `/32` would give the kernel an address and no route to use it
    /// with — the interface has to carry the subnet for anything to be
    /// reachable through it.
    #[test]
    fn the_interface_carries_the_subnet_not_one_address() {
        assert_eq!(interface_address("10.42.0.2"), "10.42.0.2/16");
        assert!(interface_address("10.42.0.2").parse::<IpAddrMask>().is_ok());
    }

    /// A node that has joined nothing and enrolled nobody has no
    /// overlay, and must not grow an interface for a network of one.
    #[tokio::test]
    async fn a_node_on_no_overlay_brings_nothing_up() {
        let database = crate::db::open_in_memory().await.expect("open");
        let config = Config::default();
        super::super::ensure_self(&database, &config)
            .await
            .expect("seeded");

        assert_eq!(ensure(&database, &config).await.expect("ensure"), None);
    }

    /// The asymmetry the whole design rests on: a node dials the peer
    /// it can reach and stays quiet towards the one it cannot, because
    /// a private node's address belongs to a NAT and guessing it would
    /// be worse than waiting to be dialled.
    #[tokio::test]
    async fn only_a_peer_with_an_address_is_dialled() {
        let database = crate::db::open_in_memory().await.expect("open");
        let config = Config::default();
        let me = super::super::ensure_self(&database, &config)
            .await
            .expect("seeded");

        // A public authority, reachable, and a private node that is not.
        for (id, kind, endpoint, address) in [
            (
                "nd-hub",
                super::super::Kind::Public,
                Some("localhost:443"),
                "10.42.0.1",
            ),
            ("nd-private", super::super::Kind::Private, None, "10.42.0.3"),
        ] {
            super::super::save(
                &database,
                &Node {
                    id: id.into(),
                    name: id.into(),
                    kind,
                    endpoint: endpoint.map(str::to_string),
                    public_key: Some(Key::generate().public_key().to_string()),
                    overlay_ip: Some(address.into()),
                    is_self: false,
                    last_seen_at: None,
                },
            )
            .await
            .expect("save");
        }

        let peers = peers(&database, &config, &me).await.expect("peers");
        assert_eq!(peers.len(), 2);

        let dialled: Vec<&Peer> = peers.iter().filter(|p| p.endpoint.is_some()).collect();
        assert_eq!(dialled.len(), 1, "both or neither were dialled");
        assert_eq!(
            dialled[0].persistent_keepalive_interval,
            Some(KEEPALIVE),
            "the node doing the dialling is the one that has to hold the NAT open"
        );
        assert!(
            peers
                .iter()
                .filter(|p| p.endpoint.is_none())
                .all(|p| p.persistent_keepalive_interval.is_none()),
            "a keepalive towards a peer with no address is packets into nowhere"
        );
    }

    /// AllowedIPs is authorisation, not just routing. A peer allowed the
    /// whole subnet could send a packet claiming to be any node on the
    /// overlay and the kernel would accept it.
    #[tokio::test]
    async fn a_peer_is_allowed_exactly_its_own_address() {
        let database = crate::db::open_in_memory().await.expect("open");
        let config = Config::default();
        let me = super::super::ensure_self(&database, &config)
            .await
            .expect("seeded");
        super::super::save(
            &database,
            &Node {
                id: "nd-hub".into(),
                name: "hub".into(),
                kind: super::super::Kind::Public,
                endpoint: None,
                public_key: Some(Key::generate().public_key().to_string()),
                overlay_ip: Some("10.42.0.1".into()),
                is_self: false,
                last_seen_at: None,
            },
        )
        .await
        .expect("save");

        let peers = peers(&database, &config, &me).await.expect("peers");
        assert_eq!(peers[0].allowed_ips.len(), 1);
        assert_eq!(peers[0].allowed_ips[0].to_string(), "10.42.0.1/32");
    }

    /// A row that has not finished joining is skipped, not refused: it
    /// is a legitimate row that WireGuard cannot yet be told about, and
    /// failing the whole overlay over one of them would take the
    /// working peers down with it.
    #[tokio::test]
    async fn a_half_joined_node_is_not_a_peer() {
        let database = crate::db::open_in_memory().await.expect("open");
        let config = Config::default();
        let me = super::super::ensure_self(&database, &config)
            .await
            .expect("seeded");

        for (id, key, address) in [
            ("nd-nokey", None, Some("10.42.0.4")),
            (
                "nd-noaddress",
                Some(Key::generate().public_key().to_string()),
                None,
            ),
        ] {
            super::super::save(
                &database,
                &Node {
                    id: id.into(),
                    name: id.into(),
                    kind: super::super::Kind::Private,
                    endpoint: None,
                    public_key: key,
                    overlay_ip: address.map(str::to_string),
                    is_self: false,
                    last_seen_at: None,
                },
            )
            .await
            .expect("save");
        }

        assert!(peers(&database, &config, &me)
            .await
            .expect("peers")
            .is_empty());
    }
}
