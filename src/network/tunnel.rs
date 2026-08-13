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
//! **What it must not do is answer that question by rebuilding.**
//! Nothing takes the interface down when this process stops: `wabot0`,
//! its address, its peers and the kernel's live sessions outlive the
//! binary, and so does the port mapping into a container, which is
//! iptables. So while this node is being replaced, packets between two
//! containers on different machines keep moving with nothing running to
//! carry them — and the only thing that can break that is this file.
//!
//! `configure_interface` breaks it: it sets `ReplacePeers`, so the
//! kernel drops every peer along with its session keys and, for a peer
//! this node did not dial, the endpoint it had *learned* from the last
//! handshake. A public node configures no endpoint for a private peer on
//! purpose — see below — so after a restart it holds a peer it cannot
//! send to and a session it cannot read, until the other end notices and
//! starts again.
//!
//! Measured between the two test nodes: every restart of this binary
//! cost the overlay 45–55 seconds of silence — 30 of drain, then up to
//! 25 more before the private node's keepalive rebuilt the session — and
//! `wal_receiver_timeout` is 60, so a database standby on the other
//! machine lost its replication stream on every single deployment. It
//! reconnected by itself twenty seconds later and nothing anywhere
//! recorded that it had happened.
//!
//! So [`needed`] compares first, and a start that changes nothing tells
//! the kernel nothing. A peer that *has* changed is set on its own,
//! which leaves the sessions of the peers either side of it alone.
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

use defguard_wireguard_rs::{host::Host, key::Key, net::IpAddrMask, peer::Peer};
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

/// What the kernel has to be told, given what it already holds.
///
/// Pure, and above the Linux line, because this is the whole of the
/// decision the module note is about: the netlink calls below are three
/// lines each and this is the part that can be wrong.
// Above the Linux line, so a machine that is not a node still compiles
// and still runs the tests for it — and on that machine `apply` is not
// there to call it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, PartialEq)]
enum Change {
    /// The interface already says this. Telling it again is what used to
    /// cost a database standby its replication stream on every restart.
    Nothing,
    /// These peers differ and these are no longer wanted. One call each,
    /// so a node joining does not disturb the sessions of the peers that
    /// were already there.
    Peers { set: Vec<Peer>, gone: Vec<Key> },
    /// The interface itself is wrong — a different key or a different
    /// port, which is this node's own identity on the overlay and not
    /// something that changes without somebody meaning it. Everything is
    /// rebuilt, sessions included, because there is nothing to preserve:
    /// a peer's session is derived from the key that just changed.
    ///
    /// It carries **why**. This is the one branch that costs a database
    /// standby on another machine its replication stream, and the first
    /// version of this decided it silently — so the node did the
    /// expensive thing on every start and the journal said only that an
    /// interface had been configured, which it says either way.
    Everything(&'static str),
}

/// Compare what the rows want against what the kernel reports.
///
/// `None` for the host is an interface that could not be read, which is
/// an interface that has not been configured — the full path covers it.
// Above the Linux line, so a machine that is not a node still compiles
// and still runs the tests for it — and on that machine `apply` is not
// there to call it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn needed(host: Option<&Host>, private: &str, port: u16, wanted: &[Peer]) -> Change {
    let Some(host) = host else {
        return Change::Everything("the interface could not be read");
    };
    // The parse failing means the stored key is not a key, and the only
    // thing that reports that usefully is the configure below trying it.
    let Ok(private) = Key::try_from(private) else {
        return Change::Everything("the stored private key is not a key");
    };
    if host.listen_port != port {
        return Change::Everything("a different listen port");
    }
    match &host.private_key {
        // Compared as public halves, never as the stored bytes.
        //
        // WireGuard **clamps** a private key when it takes it — `[0] &=
        // 248` and the top two bits of `[31]` — and `Key::generate` does
        // not, so what reads back differs from what the database holds by
        // up to three bits on every node that ever existed. The first
        // version of this compared the bytes, concluded the interface
        // belonged to somebody else and rebuilt it at every start: the
        // precise failure the comparison was added to prevent,
        // reintroduced by the comparison, and invisible until a node said
        // `reason="a different private key" keyed=true`.
        //
        // The clamp happens inside the curve multiplication anyway, so
        // both forms have the same public half. Which is also the honest
        // question — whether this is the same identity, not whether two
        // encodings match.
        Some(held) if held.public_key() != private.public_key() => {
            return Change::Everything("a different key")
        }
        Some(_) => {}
        // The read did not come back with one, which is not the same as a
        // key that differs and must not be treated as it: one is a node
        // whose identity changed, the other is the kernel — or the crate
        // reading it — declining to hand a private key back.
        //
        // A **completed handshake proves the key** anyway, and proves it
        // better than comparing bytes: the peer on the other side did it
        // against this node's public half, so an interface somebody has
        // spoken to is carrying the key the rows say it holds. Only when
        // every configured peer has never once answered is there reason to
        // suspect the interface itself — and then there is no session left
        // to lose by rebuilding it.
        None if !host.peers.is_empty()
            && host
                .peers
                .values()
                .all(|peer| peer.last_handshake.is_none()) =>
        {
            return Change::Everything("no peer has ever completed a handshake")
        }
        None => {}
    }

    let set: Vec<Peer> = wanted
        .iter()
        .filter(|peer| {
            !host
                .peers
                .get(&peer.public_key)
                .is_some_and(|k| holds(k, peer))
        })
        .cloned()
        .collect();
    let gone: Vec<Key> = host
        .peers
        .keys()
        .filter(|key| !wanted.iter().any(|peer| &peer.public_key == *key))
        .cloned()
        .collect();

    match set.is_empty() && gone.is_empty() {
        true => Change::Nothing,
        false => Change::Peers { set, gone },
    }
}

/// Whether the kernel's peer already says what this node wants it to.
///
/// Only the fields this node writes. The handshake and the counters are
/// the kernel's answer to a different question, and an endpoint this node
/// deliberately left unset is one it *learned* — comparing that against
/// `None` would call every working peer wrong and rebuild it, which is
/// the bug this function exists to avoid.
// Above the Linux line, so a machine that is not a node still compiles
// and still runs the tests for it — and on that machine `apply` is not
// there to call it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn holds(existing: &Peer, wanted: &Peer) -> bool {
    let allowed = |peer: &Peer| {
        let mut ips: Vec<String> = peer.allowed_ips.iter().map(ToString::to_string).collect();
        ips.sort();
        ips
    };
    // Nought and nothing are the same keepalive. WireGuard reads zero as
    // off and the kernel hands `0` back for a peer that was configured
    // without one, so comparing the options directly called every peer on
    // a public node different at every start — which cost a netlink write
    // that was not needed and, more to the point, made "already matches"
    // a line that never printed.
    let keepalive = |peer: &Peer| peer.persistent_keepalive_interval.unwrap_or(0);

    allowed(existing) == allowed(wanted)
        && keepalive(existing) == keepalive(wanted)
        && match wanted.endpoint {
            Some(endpoint) => existing.endpoint == Some(endpoint),
            None => true,
        }
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

    let wanted = interface_address(address)
        .parse::<IpAddrMask>()
        .map_err(|error| format!("{address} is not an address: {error}"))?;

    // Read before writing. An interface that cannot be read is one that
    // is not configured, and `needed` reads that as "everything".
    let host = api.read_interface_data().ok();

    match needed(host.as_ref(), private, port, &peers) {
        Change::Everything(reason) => {
            // At `info`, once per start: whether a deployment costs the
            // overlay a minute of silence now depends on this line.
            tracing::info!(
                reason,
                port = host.as_ref().map(|host| host.listen_port),
                keyed = host.as_ref().map(|host| host.private_key.is_some()),
                "rebuilding the overlay interface"
            );
            return api
                .configure_interface(&InterfaceConfiguration {
                    name: INTERFACE.to_string(),
                    prvkey: private.to_string(),
                    addresses: vec![wanted],
                    port,
                    peers,
                    mtu: None,
                    fwmark: None,
                })
                .map_err(|error| format!("could not configure {INTERFACE}: {error}"));
        }
        Change::Peers { set, gone } => {
            for peer in &set {
                api.configure_peer(peer)
                    .map_err(|error| format!("could not configure a peer: {error}"))?;
            }
            for key in &gone {
                api.remove_peer(key)
                    .map_err(|error| format!("could not remove a peer: {error}"))?;
            }
            tracing::info!(
                set = set.len(),
                removed = gone.len(),
                "the overlay's peers changed"
            );
        }
        // The case the module note is about, and worth a line: somebody
        // reading a deployment's journal should be able to see that the
        // overlay was left alone rather than infer it from silence.
        Change::Nothing => tracing::info!("the overlay interface already matches"),
    }

    // Whatever the peers needed, and last rather than first: the address
    // is the one part of this the kernel cannot be asked about — `Host`
    // carries WireGuard's own state and an interface address is not part
    // of it — so it is written rather than compared. netlink *replaces*
    // it instead of adding a second one, which is why doing this on every
    // start is not the thing the module note warns about.
    api.assign_address(&wanted)
        .map_err(|error| format!("could not address {INTERFACE}: {error}"))
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
    ///
    /// And a `/24`, not a `/16`: the wider mask claimed a route over
    /// every project bridge on the machine, which are carved out of the
    /// same `10.42.0.0/16`. An overlay address names a node, never a
    /// container.
    #[test]
    fn the_interface_carries_the_subnet_not_one_address() {
        assert_eq!(interface_address("10.42.0.2"), "10.42.0.2/24");
        assert!(interface_address("10.42.0.2").parse::<IpAddrMask>().is_ok());
    }

    /// Two keys and a peer built the way `peers` builds one.
    fn key(seed: u8) -> Key {
        Key::new([seed; 32])
    }

    fn peer(seed: u8, allowed: &str) -> Peer {
        let mut peer = Peer::new(key(seed));
        peer.allowed_ips = vec![allowed.parse().expect("a mask")];
        peer
    }

    fn kernel(private: &Key, port: u16, peers: Vec<Peer>) -> Host {
        let mut host = Host::new(port, private.clone());
        for peer in peers {
            host.peers.insert(peer.public_key.clone(), peer);
        }
        host
    }

    /// The one that cost a replication stream on every deployment.
    ///
    /// Nothing takes the interface down when this process stops, so a
    /// start that finds the kernel already holding what the rows say has
    /// nothing to do — and doing it anyway replaces the peers, drops
    /// their sessions and the endpoints learned from them, and the other
    /// end goes quiet for the best part of a minute.
    #[test]
    fn a_start_that_changes_nothing_tells_the_kernel_nothing() {
        let private = key(1);
        let wanted = vec![peer(2, "10.42.0.4/32")];
        let host = kernel(&private, DEFAULT_PORT, wanted.clone());

        assert_eq!(
            needed(Some(&host), &private.to_string(), DEFAULT_PORT, &wanted),
            Change::Nothing
        );
    }

    /// An endpoint this node did not configure is one the kernel
    /// *learned* from a handshake — the whole of how a public node
    /// answers a private one. Reading that as a difference would rebuild
    /// every working peer at every start, which is the failure this
    /// comparison replaced.
    #[test]
    fn an_endpoint_the_kernel_learned_is_not_a_difference() {
        let private = key(1);
        let wanted = vec![peer(2, "10.42.0.4/32")];

        let mut learned = wanted[0].clone();
        learned.endpoint = Some("203.0.113.7:51820".parse().expect("an address"));
        learned.last_handshake = Some(std::time::UNIX_EPOCH);
        learned.rx_bytes = 4096;
        let host = kernel(&private, DEFAULT_PORT, vec![learned]);

        assert_eq!(
            needed(Some(&host), &private.to_string(), DEFAULT_PORT, &wanted),
            Change::Nothing
        );
    }

    /// The kernel hands back `0` for a peer configured without a
    /// keepalive, which is what "off" is spelled as down there. Reading
    /// that as a difference from `None` made a public node rewrite every
    /// peer at every start — harmless, and it meant this file's own
    /// "already matches" never printed on the node that most needed it.
    #[test]
    fn a_keepalive_of_nought_is_no_keepalive() {
        let private = key(1);
        let mut reported = peer(2, "10.42.0.4/32");
        reported.persistent_keepalive_interval = Some(0);
        reported.endpoint = Some("203.0.113.7:51820".parse().expect("an address"));
        reported.last_handshake = Some(std::time::UNIX_EPOCH);

        let host = kernel(&private, DEFAULT_PORT, vec![reported]);
        assert_eq!(
            needed(
                Some(&host),
                &private.to_string(),
                DEFAULT_PORT,
                &[peer(2, "10.42.0.4/32")]
            ),
            Change::Nothing
        );
    }

    /// A peer that differs is set on its own, and one nobody wants is
    /// removed — so a node joining does not cost the peers already there
    /// their sessions.
    #[test]
    fn only_the_peers_that_moved_are_touched() {
        let private = key(1);
        let staying = peer(2, "10.42.0.4/32");
        let arriving = peer(3, "10.42.0.5/32");
        let leaving = peer(4, "10.42.0.6/32");
        let host = kernel(
            &private,
            DEFAULT_PORT,
            vec![staying.clone(), leaving.clone()],
        );

        let change = needed(
            Some(&host),
            &private.to_string(),
            DEFAULT_PORT,
            &[staying, arriving.clone()],
        );
        match change {
            Change::Peers { set, gone } => {
                assert_eq!(set.len(), 1, "only the new peer");
                assert_eq!(set[0].public_key, arriving.public_key);
                assert_eq!(gone, vec![leaving.public_key]);
            }
            other => panic!("expected the two peers, got {other:?}"),
        }
    }

    /// A peer whose address moved is told, rather than left pointing at
    /// where the node used to be — the comparison has to be about the
    /// fields this node writes, not about the peer existing.
    #[test]
    fn a_peer_whose_allowed_address_changed_is_told() {
        let private = key(1);
        let host = kernel(&private, DEFAULT_PORT, vec![peer(2, "10.42.0.4/32")]);

        let change = needed(
            Some(&host),
            &private.to_string(),
            DEFAULT_PORT,
            &[peer(2, "10.42.0.9/32")],
        );
        assert!(
            matches!(&change, Change::Peers { set, gone } if set.len() == 1 && gone.is_empty()),
            "expected the one peer to be set, got {change:?}"
        );
    }

    /// This node's own identity on the overlay. There is nothing to
    /// preserve when the key changes — every session was derived from the
    /// old one — and a different port is an interface nobody can reach.
    #[test]
    fn a_different_key_or_port_rebuilds_the_interface() {
        let private = key(1);
        let wanted = vec![peer(2, "10.42.0.4/32")];

        let elsewhere = kernel(&key(9), DEFAULT_PORT, wanted.clone());
        assert_eq!(
            needed(
                Some(&elsewhere),
                &private.to_string(),
                DEFAULT_PORT,
                &wanted
            ),
            Change::Everything("a different key"),
            "a key that is not this node's"
        );

        let other_port = kernel(&private, 51821, wanted.clone());
        assert_eq!(
            needed(
                Some(&other_port),
                &private.to_string(),
                DEFAULT_PORT,
                &wanted
            ),
            Change::Everything("a different listen port"),
            "a port nobody is dialling"
        );

        assert_eq!(
            needed(None, &private.to_string(), DEFAULT_PORT, &wanted),
            Change::Everything("the interface could not be read"),
            "an interface that could not be read is one that is not there"
        );
    }

    /// The one the nodes found. WireGuard clamps a private key when it
    /// takes it, so what the kernel hands back is never quite the value in
    /// the database — and comparing the bytes rebuilt a perfectly healthy
    /// interface at every start, on both test nodes, which is the failure
    /// this whole comparison exists to prevent.
    #[test]
    fn a_key_the_kernel_clamped_is_the_same_key() {
        let stored = key(1);

        let mut clamped = stored.as_array();
        clamped[0] &= 248;
        clamped[31] = (clamped[31] & 127) | 64;
        assert_ne!(clamped, stored.as_array(), "the clamp has to change it");

        let host = kernel(
            &Key::new(clamped),
            DEFAULT_PORT,
            vec![peer(2, "10.42.0.4/32")],
        );
        assert_eq!(
            needed(
                Some(&host),
                &stored.to_string(),
                DEFAULT_PORT,
                &[peer(2, "10.42.0.4/32")]
            ),
            Change::Nothing
        );
    }

    /// A read that comes back without a private key is not a node whose
    /// identity changed, and treating it as one rebuilt a healthy
    /// interface at every start — which is the whole failure this file is
    /// about. A peer that has completed a handshake did it against this
    /// node's public half, so the key is proved without being read.
    #[test]
    fn a_handshake_proves_the_key_the_kernel_did_not_hand_back() {
        let private = key(1);
        let mut answered = peer(2, "10.42.0.4/32");
        answered.last_handshake = Some(std::time::UNIX_EPOCH);

        let mut host = kernel(&private, DEFAULT_PORT, vec![answered.clone()]);
        host.private_key = None;

        assert_eq!(
            needed(
                Some(&host),
                &private.to_string(),
                DEFAULT_PORT,
                &[peer(2, "10.42.0.4/32")]
            ),
            Change::Nothing
        );
    }

    /// And when no peer has ever answered, the interface is the thing to
    /// suspect — there is no session to lose by rebuilding it.
    #[test]
    fn an_interface_no_peer_has_ever_answered_is_rebuilt() {
        let private = key(1);
        let mut host = kernel(&private, DEFAULT_PORT, vec![peer(2, "10.42.0.4/32")]);
        host.private_key = None;

        assert_eq!(
            needed(
                Some(&host),
                &private.to_string(),
                DEFAULT_PORT,
                &[peer(2, "10.42.0.4/32")]
            ),
            Change::Everything("no peer has ever completed a handshake")
        );
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
                    allows: Vec::new(),
                    ca_pem: None,
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
                allows: Vec::new(),
                ca_pem: None,
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
                    allows: Vec::new(),
                    ca_pem: None,
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
