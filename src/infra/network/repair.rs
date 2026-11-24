//! A tunnel that dies comes back: `docs/network.md`, phase 13.
//!
//! On 2026-11-23 node-l1 lost node-0 for an afternoon and only a packet
//! sent by hand brought it back. The home router had stopped passing one
//! pair of addresses and ports, `:51820 → node-0:51820`, while every other
//! pair went through, and nothing on either node would ever change that
//! pair. Before that, nothing re-ran `tunnel::ensure` unless a census
//! changed a row, so a node that started before its network never dialled
//! its authority at all, and an authority that moved was dialled at its old
//! address until a restart.
//!
//! So one loop, every thirty seconds:
//!
//! - **It converges.** `tunnel::converge` every pass: the interface is
//!   recreated if something deleted it, every dialled name is resolved
//!   again, and a peer missing from the kernel is put back. A pass that
//!   finds the kernel right tells it nothing, which is what lets this run
//!   this often.
//! - **The side that dials repairs its path.** A dialled tunnel that died
//!   is tried at the next address its name has, at once and every three
//!   minutes after. After three minutes, a node behind NAT takes a new
//!   listen port — a new pair at the router — at most once every five.
//! - **The side that waits forgets.** An endpoint it learned from a tunnel
//!   that has been dead for three minutes is dropped, which also forgets
//!   the dialler's last timestamp: the only cure for a clock that went back.
//! - **It says so**, per peer, in `tunnel_repair`, which the attention card
//!   and `doctor` read beside the stale tunnel.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use wabot::sqlite::{SqliteDatabase, SqliteResult};

use super::lan::Held;
use super::tunnel::{self, Interface, Outcome, Path, STALE_AFTER};
use crate::infra::config::Config;

/// How often the loop looks. A fifth of WireGuard's own 180 seconds, so a
/// dead tunnel is seen well inside the time it takes to be declared dead.
pub const EVERY: Duration = Duration::from_secs(30);

/// How long a dialled tunnel has been dead before a node behind NAT moves.
/// WireGuard's own figure again: by then its keys are gone, and the dialler
/// has sent an initiation every five seconds for all of it.
pub const MOVE_AFTER: u64 = 180;

/// The least time between two moves. A move costs every LAN sibling its
/// session for a moment, and the new port reaches them on the next census.
pub const MOVE_AT_MOST_EVERY: u64 = 300;

/// How long a tunnel has been dead before its next address is tried again.
pub const DIAL_AGAIN_EVERY: u64 = 180;

/// How old a learned endpoint's handshake is when the side that waits
/// drops it: three minutes after it went stale.
pub const FORGET_AFTER: u64 = STALE_AFTER + 180;

/// How recently a peer must have answered over HTTPS for a dead tunnel to
/// be the path's fault rather than the machine's. Four passes of the
/// collector, which reaches an authority every fifteen seconds.
pub const ANSWERED_WITHIN: u64 = 60;

/// One peer, as the judgement needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub node_id: String,
    pub name: String,
    pub public_key: String,
    pub path: Path,
    /// This node sends it a keepalive, which is to say dials it across the
    /// internet: the one kind of path a new port can mend.
    pub keepalive: bool,
    /// It answered this node over HTTPS lately, so the machine is up and
    /// the tunnel is what is broken. Without this, an authority switched
    /// off for an hour would be met with twelve moves, each costing the
    /// LAN siblings their sessions, with no census to tell them the port.
    pub answers: bool,
}

/// What the judgement asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Resolve its name again and dial the address after the one held.
    /// `first` is the start of a dead stretch, which is the one worth
    /// recording: the later ones would bury a move under themselves.
    DialAgain {
        node_id: String,
        public_key: String,
        first: bool,
    },
    /// Move this node to a new listen port, because of this peer.
    Move { node_id: String, name: String },
    /// Drop the endpoint learned from this peer.
    Forget {
        node_id: String,
        name: String,
        public_key: String,
    },
}

/// What the loop remembers between passes.
#[derive(Debug, Default)]
pub struct Memory {
    /// Dialled peers whose tunnel is dead, by public key.
    dead: BTreeMap<String, Dead>,
    moved_at: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct Dead {
    since: u64,
    dialled_again: u64,
}

/// What to do this pass.
///
/// Pure, with the clock and the facts handed in, because this is where all
/// of the judgement is and none of it needs a kernel. `may_move` is this
/// node's side of the move — behind NAT, dialled by nobody at its name, and
/// every LAN sibling able to follow — and each peer brings its own.
pub fn judge(memory: &mut Memory, peers: &[Peer], now: u64, may_move: bool) -> Vec<Act> {
    let mut acts = Vec::new();
    let mut still_dead = BTreeSet::new();
    let mut moving = false;

    for peer in peers {
        let since = match (&peer.path, peer.keepalive) {
            (Path::Stale { ago, .. }, true) => now.saturating_sub(ago.saturating_sub(STALE_AFTER)),
            (Path::Dialling { .. }, true) => memory
                .dead
                .get(&peer.public_key)
                .map_or(now, |dead| dead.since),
            (Path::Stale { ago, .. }, false) => {
                if *ago >= FORGET_AFTER {
                    acts.push(Act::Forget {
                        node_id: peer.node_id.clone(),
                        name: peer.name.clone(),
                        public_key: peer.public_key.clone(),
                    });
                }
                continue;
            }
            // A pair on one LAN that has never answered is mended by
            // converging, which every pass does; and every other path is
            // either working or nobody's to dial.
            _ => continue,
        };
        still_dead.insert(peer.public_key.clone());

        let first = !memory.dead.contains_key(&peer.public_key);
        let dead = memory.dead.entry(peer.public_key.clone()).or_insert(Dead {
            since,
            dialled_again: 0,
        });
        dead.since = since;
        if first || now.saturating_sub(dead.dialled_again) >= DIAL_AGAIN_EVERY {
            dead.dialled_again = now;
            acts.push(Act::DialAgain {
                node_id: peer.node_id.clone(),
                public_key: peer.public_key.clone(),
                first,
            });
        }

        let rested = memory
            .moved_at
            .is_none_or(|at| now.saturating_sub(at) >= MOVE_AT_MOST_EVERY);
        if may_move
            && peer.answers
            && !moving
            && rested
            && now.saturating_sub(dead.since) >= MOVE_AFTER
        {
            moving = true;
            acts.push(Act::Move {
                node_id: peer.node_id.clone(),
                name: peer.name.clone(),
            });
        }
    }

    if moving {
        memory.moved_at = Some(now);
    }
    memory.dead.retain(|key, _| still_dead.contains(key));
    acts
}

/// Whether this machine holds an address the internet can reach directly.
///
/// A node that does is dialled at its port and keeps it; one that does not
/// is behind somebody's translation, which is where a stale pair lives.
/// Shared address space (`100.64.0.0/10`) is a carrier's NAT, so it counts
/// as behind one.
pub fn holds_a_public_address(held: &[Held]) -> bool {
    held.iter().any(|held| {
        let address = held.address;
        let [first, second, ..] = address.octets();
        let shared = first == 100 && (64..128).contains(&second);
        !(address.is_private()
            || shared
            || address.is_loopback()
            || address.is_link_local()
            || address.is_unspecified()
            || address.is_broadcast())
    })
}

/// Whether every node on this machine's own networks has said its port.
///
/// Those are the ones that dial this node at its port rather than learning
/// it from a handshake, and one that has not said its own is older than
/// the field: it would go on dialling the configured port, and the move
/// would cut it off. Saying its port is the version gate, read off the
/// same fact that makes the move safe.
pub fn siblings_can_follow(
    lan: &BTreeMap<String, Vec<std::net::Ipv4Addr>>,
    held: &[Held],
    ports: &BTreeMap<String, u16>,
) -> bool {
    lan.iter()
        .filter(|(_, addresses)| super::lan::candidate(addresses, held).is_some())
        .all(|(node, _)| ports.contains_key(node))
}

/// Note that a node answered this one over HTTPS.
///
/// Called by the collector when an authority hands back its errands. Held
/// in memory: it is this process's own observation, read by this process's
/// own loop, and a restart that forgets it costs one minute's patience.
pub fn answered(node_id: &str) {
    if let Ok(mut heard) = HEARD.lock() {
        heard.insert(node_id.to_string(), now_seconds());
    }
}

fn answered_lately(node_id: &str, now: u64) -> bool {
    HEARD
        .lock()
        .ok()
        .and_then(|heard| heard.get(node_id).copied())
        .is_some_and(|at| now.saturating_sub(at) <= ANSWERED_WITHIN)
}

static HEARD: std::sync::Mutex<BTreeMap<String, u64>> = std::sync::Mutex::new(BTreeMap::new());

/// The last thing done about a tunnel to one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repair {
    /// Milliseconds since the epoch.
    pub at: i64,
    pub did: String,
}

/// What was last done about each node's tunnel, by node id.
pub async fn last(database: &SqliteDatabase) -> SqliteResult<BTreeMap<String, Repair>> {
    database
        .read(|connection| {
            let mut statement =
                connection.prepare("SELECT \"node_id\", \"at\", \"did\" FROM tunnel_repair")?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    Repair {
                        at: row.get(1)?,
                        did: row.get(2)?,
                    },
                ))
            })?;
            rows.collect()
        })
        .await
}

async fn record(database: &SqliteDatabase, node_id: &str, did: String) {
    let node_id = node_id.to_string();
    let written = database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO tunnel_repair (\"node_id\", \"at\", \"did\") VALUES (?1, ?2, ?3) \
                 ON CONFLICT (\"node_id\") DO UPDATE SET \
                   \"at\" = excluded.\"at\", \"did\" = excluded.\"did\"",
                (node_id, crate::infra::clock::now_ms(), did),
            )?;
            Ok(())
        })
        .await;
    if let Err(error) = written {
        tracing::warn!(%error, "could not record what was done about a tunnel");
    }
}

/// The loop, beside the listeners in `serve`.
pub async fn loop_forever(
    database: std::sync::Arc<SqliteDatabase>,
    config: Config,
    cancel: wabot::lifecycle::Cancel,
) -> anyhow::Result<()> {
    let mut memory = Memory::default();
    let mut failing: Option<String> = None;
    loop {
        // After the wait rather than before it: `serve` has just run
        // `ensure` on its way up.
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(EVERY) => {}
        }
        pass(&database, &config, &mut memory, &mut failing).await;
    }
}

async fn pass(
    database: &SqliteDatabase,
    config: &Config,
    memory: &mut Memory,
    failing: &mut Option<String>,
) {
    let Ok(Some(me)) = super::me(database).await else {
        return;
    };
    if me.overlay_ip.is_none() {
        return;
    }
    let nodes = super::all(database).await.unwrap_or_default();
    let held = super::lan::held();
    // An interface that cannot be read has no peers to judge, and the
    // converge below is what recreates it.
    let interface = tunnel::observed().unwrap_or(Interface {
        port: 0,
        peers: Vec::new(),
    });
    let now = now_seconds();

    let peers: Vec<Peer> = tunnel::pair(&me, &nodes, &interface, &held)
        .into_iter()
        .filter_map(|paired| {
            let public_key = nodes
                .iter()
                .find(|node| node.id == paired.node_id)?
                .public_key
                .clone()?;
            let keepalive = interface
                .peers
                .iter()
                .find(|peer| peer.public_key == public_key)
                .is_some_and(|peer| peer.keepalive > 0);
            Some(Peer {
                answers: answered_lately(&paired.node_id, now),
                node_id: paired.node_id,
                name: paired.name,
                public_key,
                path: paired.path,
                keepalive,
            })
        })
        .collect();

    let dead = peers.iter().any(|peer| {
        peer.keepalive && matches!(peer.path, Path::Stale { .. } | Path::Dialling { .. })
    });
    let may_move = dead && may_move(database, &me, &held).await;

    let mut away = BTreeSet::new();
    for act in judge(memory, &peers, now, may_move) {
        match act {
            Act::DialAgain {
                node_id,
                public_key,
                first,
            } => {
                away.insert(public_key);
                if first {
                    record(database, &node_id, "dialled it again".into()).await;
                }
            }
            Act::Move { node_id, name } => {
                let from = super::port::own(database, config).await;
                let to = super::port::another(from, super::port::entropy());
                if let Err(error) = super::port::move_to(database, to).await {
                    tracing::warn!(%error, "could not move the overlay to a new port");
                    continue;
                }
                tracing::warn!(
                    from,
                    to,
                    peer = %name,
                    "the tunnel has been dead for three minutes; moving to a new port"
                );
                record(database, &node_id, format!("moved to port {to}")).await;
            }
            Act::Forget {
                node_id,
                name,
                public_key,
            } => match tunnel::forget(std::slice::from_ref(&public_key)) {
                Ok(()) => {
                    tracing::info!(peer = %name, "forgot the address a dead tunnel had taught");
                    record(
                        database,
                        &node_id,
                        "forgot the address it had learned".into(),
                    )
                    .await;
                }
                Err(error) => {
                    tracing::warn!(%error, peer = %name, "could not forget a dead tunnel")
                }
            },
        }
    }

    match tunnel::converge(database, config, &away).await {
        Ok(Some((_, outcome))) => {
            if failing.take().is_some() {
                tracing::info!("the overlay can be configured again");
            }
            if outcome != Outcome::Nothing {
                tracing::info!(?outcome, "the repair pass changed the overlay");
            }
        }
        Ok(None) => {}
        // Once per reason. A kernel that refuses WireGuard refuses it
        // every thirty seconds, and the console is where that is said.
        Err(error) => {
            let reason = error.to_string();
            if failing.as_deref() != Some(reason.as_str()) {
                tracing::warn!(%reason, "the repair pass could not configure the overlay");
                *failing = Some(reason);
            }
        }
    }
}

/// This node's side of a move: behind NAT, dialled by nobody at its name,
/// and every LAN sibling able to follow.
///
/// **Not "has no endpoint".** node-l1 has one, a name that resolves
/// nowhere, and it is the node this was written for. A name that resolves
/// is one somebody dials, maybe through a port forwarded on a router this
/// node cannot see, and a move would break it.
async fn may_move(database: &SqliteDatabase, me: &super::Node, held: &[Held]) -> bool {
    if holds_a_public_address(held) {
        return false;
    }
    if let Some(host) = me
        .endpoint
        .as_deref()
        .and_then(|endpoint| endpoint.rsplit_once(':'))
        .map(|(host, _)| host)
    {
        if tokio::net::lookup_host((host, 0)).await.is_ok() {
            return false;
        }
    }
    let (Ok(lan), Ok(ports)) = (
        super::lan::of_every_node(database).await,
        super::port::of_every_node(database).await,
    ) else {
        return false;
    };
    siblings_can_follow(&lan, held, &ports)
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(path: Path, keepalive: bool) -> Peer {
        Peer {
            node_id: "nd-l19cOD2fOUG8".into(),
            name: "node-0".into(),
            public_key: "theirs".into(),
            path,
            keepalive,
            answers: true,
        }
    }

    fn dialling() -> Path {
        Path::Dialling {
            at: "209.50.48.176:51820".into(),
        }
    }

    fn moves(acts: &[Act]) -> usize {
        acts.iter()
            .filter(|act| matches!(act, Act::Move { .. }))
            .count()
    }

    /// The start before the network, the authority that moved and the
    /// `AAAA` are all mended by dialling again, so it happens on the pass
    /// that sees the tunnel dead — and then every three minutes, not every
    /// thirty seconds, which would hop between two addresses faster than a
    /// handshake can complete on either.
    #[test]
    fn a_dialled_tunnel_that_died_is_dialled_again_at_once_and_then_now_and_again() {
        let mut memory = Memory::default();
        let dead = [peer(dialling(), true)];

        let acts = judge(&mut memory, &dead, 1_000, false);
        assert!(matches!(
            acts.as_slice(),
            [Act::DialAgain { first: true, .. }]
        ));
        assert!(judge(&mut memory, &dead, 1_030, false).is_empty());
        assert!(matches!(
            judge(&mut memory, &dead, 1_000 + DIAL_AGAIN_EVERY, false).as_slice(),
            [Act::DialAgain { first: false, .. }]
        ));
    }

    /// node-l1 on 2026-11-23: dialling node-0 from `:51820` and never
    /// answered, while another source port went straight through. After
    /// three minutes it moves, and not again for five.
    #[test]
    fn a_node_behind_nat_moves_after_three_minutes_and_not_again_for_five() {
        let mut memory = Memory::default();
        let dead = [peer(dialling(), true)];

        assert_eq!(moves(&judge(&mut memory, &dead, 1_000, true)), 0);
        assert_eq!(
            moves(&judge(&mut memory, &dead, 1_000 + MOVE_AFTER - 30, true)),
            0
        );
        assert_eq!(
            moves(&judge(&mut memory, &dead, 1_000 + MOVE_AFTER, true)),
            1
        );
        let moved = 1_000 + MOVE_AFTER;
        assert_eq!(moves(&judge(&mut memory, &dead, moved + 30, true)), 0);
        assert_eq!(
            moves(&judge(
                &mut memory,
                &dead,
                moved + MOVE_AT_MOST_EVERY - 30,
                true
            )),
            0
        );
        assert_eq!(
            moves(&judge(&mut memory, &dead, moved + MOVE_AT_MOST_EVERY, true)),
            1
        );
    }

    /// A stale tunnel has been dead since its handshake was 180 seconds
    /// old, which the kernel says — so after a restart the clock does not
    /// start again from nothing.
    #[test]
    fn a_stale_tunnel_has_been_dead_since_its_keys_went() {
        let mut memory = Memory::default();
        let stale = [peer(
            Path::Stale {
                ago: STALE_AFTER + MOVE_AFTER,
                at: None,
            },
            true,
        )];
        assert_eq!(moves(&judge(&mut memory, &stale, 5_000, true)), 1);
    }

    /// Three ways a move would be wrong, and each refuses it: a node that
    /// may not move — public, dialled at its name, or with a sibling that
    /// cannot follow — a peer that is not answering anywhere, which is a
    /// machine that is off and not a path that is stuck, and a pair on one
    /// LAN, which a new port does nothing for.
    #[test]
    fn nothing_moves_where_a_move_would_mend_nothing() {
        let late = 1_000 + MOVE_AFTER;

        let mut memory = Memory::default();
        let dead = [peer(dialling(), true)];
        judge(&mut memory, &dead, 1_000, false);
        assert_eq!(moves(&judge(&mut memory, &dead, late, false)), 0);

        let mut memory = Memory::default();
        let off = [Peer {
            answers: false,
            ..peer(dialling(), true)
        }];
        judge(&mut memory, &off, 1_000, true);
        assert_eq!(moves(&judge(&mut memory, &off, late, true)), 0);

        let mut memory = Memory::default();
        let lan = [peer(
            Path::Dialling {
                at: "192.168.1.55:51820".into(),
            },
            false,
        )];
        assert!(judge(&mut memory, &lan, 1_000, true).is_empty());
        assert!(judge(&mut memory, &lan, late, true).is_empty());
    }

    /// node-0's half: it knocked at `45.238.144.77:51820` every five
    /// seconds for 94 minutes. Three minutes after the tunnel went stale it
    /// lets go, and not before.
    #[test]
    fn the_side_that_waits_forgets_three_minutes_after_the_tunnel_went_stale() {
        let mut memory = Memory::default();
        let at = Some("45.238.144.77:51820".to_string());
        let waiting = |ago| {
            [peer(
                Path::Stale {
                    ago,
                    at: at.clone(),
                },
                false,
            )]
        };

        assert!(judge(&mut memory, &waiting(FORGET_AFTER - 30), 1_000, true).is_empty());
        assert!(matches!(
            judge(&mut memory, &waiting(FORGET_AFTER), 1_030, true).as_slice(),
            [Act::Forget { .. }]
        ));
    }

    /// A tunnel that came back is forgotten as dead, so the next time it
    /// dies the three minutes start from then.
    #[test]
    fn a_tunnel_that_came_back_starts_its_three_minutes_again() {
        let mut memory = Memory::default();
        let dead = [peer(dialling(), true)];
        let back = [peer(
            Path::Session {
                ago: 5,
                at: Some("209.50.48.176:51820".into()),
            },
            true,
        )];

        judge(&mut memory, &dead, 1_000, true);
        assert!(judge(&mut memory, &back, 1_100, true).is_empty());
        assert_eq!(
            moves(&judge(&mut memory, &dead, 1_000 + MOVE_AFTER, true)),
            0
        );
    }

    fn held(address: &str) -> Held {
        Held {
            address: address.parse().expect("an address"),
            mask: "255.255.255.0".parse().expect("a mask"),
        }
    }

    #[test]
    fn only_an_address_the_internet_reaches_is_public() {
        assert!(!holds_a_public_address(&[held("192.168.1.202")]));
        assert!(!holds_a_public_address(&[held("100.72.4.9")]));
        assert!(!holds_a_public_address(&[]));
        assert!(holds_a_public_address(&[
            held("10.0.0.4"),
            held("209.50.48.176")
        ]));
    }

    /// A sibling on this LAN that has not said its port dials this node at
    /// its own configured one, so moving would cut it off. A node on
    /// another network does not dial this one at all.
    #[test]
    fn a_node_moves_only_when_every_sibling_on_its_lan_can_follow() {
        let mine = [held("192.168.1.202")];
        let lan: BTreeMap<String, Vec<std::net::Ipv4Addr>> = [
            (
                "nd-sibling".to_string(),
                vec!["192.168.1.55".parse().unwrap()],
            ),
            ("nd-far".to_string(), vec!["10.8.0.3".parse().unwrap()]),
        ]
        .into_iter()
        .collect();

        assert!(!siblings_can_follow(&lan, &mine, &BTreeMap::new()));
        let said: BTreeMap<String, u16> = [("nd-sibling".to_string(), 51820)].into_iter().collect();
        assert!(siblings_can_follow(&lan, &mine, &said));
    }
}
