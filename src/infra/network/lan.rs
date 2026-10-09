//! What addresses this machine holds, and which of a peer's are reachable
//! from here.
//!
//! ## Why the node has to know this at all
//!
//! Three things wanted the same answer, and each had been working around
//! not having it:
//!
//! - **A console nobody could open.** A freshly installed private node
//!   answers on `localhost` and on the name derived from its id, and on
//!   nothing else — so the operator sitting at another machine on the same
//!   LAN had to *name the node after its own IP address* to get in. The
//!   proxy answers on these addresses now; see `deploy::routing`.
//! - **The certificate for those addresses.** `rcgen` emits an IP SAN for
//!   any name that parses as an address, which is what a browser wants for
//!   `https://192.168.1.10` and what a DNS SAN spelled the same way is
//!   not. See `edge::certs::own_names`.
//! - **Two private nodes on one LAN.** Neither has an endpoint the other
//!   could dial — `endpoint` is set only for a public node, by an
//!   invariant every writer keeps — so on the internet they have no path
//!   and that is accepted. On a LAN they are one hop apart, and the only
//!   thing missing was knowing it.
//!
//! ## `getifaddrs`, and not a crate for it
//!
//! `libc` is already a dependency and declares `getifaddrs` for
//! `linux_like` and for `bsd`, so one walker serves the musl node and the
//! developer's Mac — which is what lets the pure half be exercised
//! locally. The alternative, `/proc/net/route`, gives the connected
//! subnets but not this machine's address *within* them, and that address
//! is the thing that has to be reported.
//!
//! ## What is deliberately not here
//!
//! **The platform's own space.** `wabot0` carries `10.42.0.x` and every
//! project bridge carves a `/24` out of the same `10.42.0.0/16` — see
//! `network::overlay`. Left in, every consumer would be wrong in its own
//! way: the proxy would answer for a bridge gateway, so any container
//! could open the node's console; and every peer's overlay address would
//! look like it was "on a subnet this machine has", which would point each
//! peer's tunnel at its own address inside the tunnel.
//!
//! IPv6 is left out for now. A link-local address is not routable without
//! its scope and a ULA is a question of its own; both would need a wider
//! answer than "is it in my subnet".

use std::net::Ipv4Addr;

/// One address this machine holds, with the mask that says how much of it
/// is the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    pub address: Ipv4Addr,
    pub mask: Ipv4Addr,
}

/// The first two octets of everything the platform allocates for itself.
const PLATFORM: (u8, u8) = (10, 42);

/// Every address this machine holds that is worth telling anybody about.
///
/// Up, not loopback, IPv4, and outside the platform's own space — see the
/// module docs for why each of those is not a detail.
pub fn held() -> Vec<Held> {
    #[cfg(unix)]
    {
        walk().into_iter().filter(worth_naming).collect()
    }
    #[cfg(not(unix))]
    {
        Vec::new()
    }
}

/// The addresses alone, for a caller that wants names rather than
/// subnets: the routes the proxy answers on, the names the node's own
/// certificate covers, and what this node reports about itself.
pub fn addresses() -> Vec<String> {
    held()
        .into_iter()
        .map(|held| held.address.to_string())
        .collect()
}

/// Store what a node said its own addresses are.
///
/// Its own statement, like `endpoint` and `allows` beside it: nothing here
/// can see another machine's interfaces. An **empty** list is news and
/// clears the column — a node that has no address worth naming has said so
/// — where an absent field never reaches this function at all.
///
/// Its own statement, and its own column: `network::save` writes the whole
/// row and `ensure_self` calls it, so a machine's addresses must not be
/// reachable from there.
pub async fn remember(
    database: &wabot::sqlite::SqliteDatabase,
    node_id: &str,
    addresses: &[String],
) -> wabot::sqlite::SqliteResult<()> {
    let (node_id, said) = (node_id.to_string(), addresses.join(","));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE node SET \"lan\" = ?2 WHERE \"id\" = ?1",
                (node_id, said),
            )?;
            Ok(())
        })
        .await
}

/// What every node said its addresses are, by node id.
///
/// One read for the whole peer list: `tunnel::peers` asks once and looks
/// each peer up, rather than a query per peer on a pass that runs at every
/// change.
pub async fn of_every_node(
    database: &wabot::sqlite::SqliteDatabase,
) -> wabot::sqlite::SqliteResult<std::collections::BTreeMap<String, Vec<Ipv4Addr>>> {
    database
        .read(|connection| {
            let mut statement = connection.prepare(
                "SELECT \"id\", \"lan\" FROM node WHERE \"lan\" IS NOT NULL AND \"lan\" != ''",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut held = std::collections::BTreeMap::new();
            for row in rows {
                let (id, said) = row?;
                held.insert(id, parse(&said));
            }
            Ok(held)
        })
        .await
}

/// Which of a peer's addresses this machine could reach directly, if any.
///
/// Pure, because this is where all the judgement is and none of it needs a
/// kernel. Three refusals, and each is a bug that would otherwise be found
/// on somebody's network:
///
/// - **A mask of nought** matches everything. Nothing produces one today —
///   `getifaddrs` reports interface masks — and the guard is here for the
///   day a second source arrives and a default route walks in as
///   `0.0.0.0/0`.
/// - **The platform's own space**, which `held` already drops on the way
///   in. Asserted again because every peer's *overlay* address is inside a
///   subnet this machine has, so without it each peer's tunnel would be
///   pointed at that peer's address inside the tunnel.
/// - **An address this machine holds itself.** Two sites that both number
///   from `.10` would otherwise have a node dial itself.
///
/// The lowest match wins when there are several, so both ends of a pair
/// choose the same address without talking about it.
pub fn candidate(peer: &[Ipv4Addr], mine: &[Held]) -> Option<Ipv4Addr> {
    let mut reachable: Vec<Ipv4Addr> = peer
        .iter()
        .copied()
        .filter(|address| !is_platform(*address))
        .filter(|address| !mine.iter().any(|held| held.address == *address))
        .filter(|address| {
            mine.iter()
                .any(|held| held.mask != Ipv4Addr::UNSPECIFIED && same_network(*address, held))
        })
        .collect();
    reachable.sort();
    reachable.into_iter().next()
}

fn same_network(address: Ipv4Addr, held: &Held) -> bool {
    let (address, mine, mask) = (
        u32::from(address),
        u32::from(held.address),
        u32::from(held.mask),
    );
    address & mask == mine & mask
}

/// Addresses as they arrive from a report or a census.
///
/// Anything unreadable is dropped rather than refused: this is another
/// machine's list, and one bad entry must not lose the rest.
pub fn parse(said: &str) -> Vec<Ipv4Addr> {
    said.split(',')
        .filter_map(|address| address.trim().parse().ok())
        .collect()
}

fn is_platform(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    (octets[0], octets[1]) == PLATFORM
}

fn worth_naming(held: &Held) -> bool {
    !held.address.is_loopback()
        && !held.address.is_link_local()
        && !held.address.is_multicast()
        && !held.address.is_unspecified()
        && !held.address.is_broadcast()
        && !is_platform(held.address)
}

/// Every IPv4 address on an interface that is up, from the kernel.
///
/// The one `unsafe` in this module, and it is a pointer walk: `getifaddrs`
/// allocates a linked list and `freeifaddrs` frees it, so the only rule is
/// that nothing escapes the loop except copies.
#[cfg(unix)]
fn walk() -> Vec<Held> {
    let mut found = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();

    // SAFETY: `getifaddrs` writes a pointer to a list it owns, or returns
    // non-zero and writes nothing. Nothing is read unless it succeeded.
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return found;
    }

    let mut entry = list;
    while !entry.is_null() {
        // SAFETY: the list is non-null here and `ifa_next` is either the
        // next entry or null, which ends the loop.
        let current = unsafe { &*entry };
        entry = current.ifa_next;

        let flags = current.ifa_flags as i32;
        if flags & libc::IFF_UP == 0 || flags & libc::IFF_LOOPBACK != 0 {
            continue;
        }
        let (Some(address), Some(mask)) = (ipv4_of(current.ifa_addr), ipv4_of(current.ifa_netmask))
        else {
            continue;
        };
        found.push(Held { address, mask });
    }

    // SAFETY: `list` came from `getifaddrs` and is freed exactly once.
    unsafe { libc::freeifaddrs(list) };
    found
}

/// The address behind a `sockaddr`, when it is an IPv4 one.
#[cfg(unix)]
fn ipv4_of(sockaddr: *const libc::sockaddr) -> Option<Ipv4Addr> {
    if sockaddr.is_null() {
        return None;
    }
    // SAFETY: non-null, and only `sa_family` is read before the cast —
    // which is the field every `sockaddr` variant starts with.
    let family = unsafe { (*sockaddr).sa_family } as i32;
    if family != libc::AF_INET {
        return None;
    }
    // SAFETY: the family says this is a `sockaddr_in`.
    let inet = unsafe { &*(sockaddr as *const libc::sockaddr_in) };
    // `s_addr` is network byte order; `Ipv4Addr::from` takes host order.
    Some(Ipv4Addr::from(u32::from_be(inet.sin_addr.s_addr)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(address: &str, mask: &str) -> Held {
        Held {
            address: address.parse().expect("an address"),
            mask: mask.parse().expect("a mask"),
        }
    }

    /// Two private nodes on one LAN can reach each other, and that is the
    /// whole point of this module.
    #[test]
    fn a_peer_on_this_machines_lan_is_reachable() {
        let mine = vec![at("192.168.1.10", "255.255.255.0")];
        assert_eq!(
            candidate(&parse("192.168.1.20"), &mine),
            Some("192.168.1.20".parse().expect("an address"))
        );
        // And one somewhere else is not, however private it looks.
        assert_eq!(candidate(&parse("10.8.0.4"), &mine), None);
        assert_eq!(candidate(&parse("192.168.2.20"), &mine), None);
    }

    /// The mask is read, not assumed.
    ///
    /// A `/16` LAN reaches addresses a `/24` would refuse, and guessing
    /// the wrong one either loses a real path or invents one.
    #[test]
    fn the_mask_decides_and_not_the_first_three_octets() {
        let wide = vec![at("172.16.4.10", "255.255.0.0")];
        assert!(candidate(&parse("172.16.9.20"), &wide).is_some());

        let narrow = vec![at("172.16.4.10", "255.255.255.0")];
        assert_eq!(candidate(&parse("172.16.9.20"), &narrow), None);
    }

    /// **The refusal that saves the overlay from itself.**
    ///
    /// `wabot0` carries `10.42.0.x` and every project bridge carves a
    /// `/24` out of the same `/16`, so every peer's overlay address is
    /// "on a subnet this machine has". Without this the tunnel to each
    /// peer would be pointed at that peer's address *inside* the tunnel.
    #[test]
    fn no_peer_is_ever_reached_at_a_platform_address() {
        let mine = vec![at("10.42.0.1", "255.255.255.0")];
        assert_eq!(candidate(&parse("10.42.0.7"), &mine), None);

        // And a bridge gateway is not a way to reach anybody either.
        let bridge = vec![at("10.42.2.1", "255.255.255.0")];
        assert_eq!(candidate(&parse("10.42.2.254"), &bridge), None);
    }

    /// Two sites that both number from `.10` must not make a node dial
    /// itself.
    #[test]
    fn an_address_this_machine_holds_is_not_a_way_to_reach_somebody_else() {
        let mine = vec![at("192.168.1.10", "255.255.255.0")];
        assert_eq!(candidate(&parse("192.168.1.10"), &mine), None);
    }

    /// A mask of nought matches everything, so it is nothing.
    #[test]
    fn a_mask_of_nothing_does_not_put_the_world_on_this_lan() {
        let mine = vec![at("192.168.1.10", "0.0.0.0")];
        assert_eq!(candidate(&parse("203.0.113.9"), &mine), None);
    }

    /// The lowest match, so both ends of a pair pick the same one without
    /// talking about it.
    #[test]
    fn the_choice_is_the_same_on_both_sides() {
        let mine = vec![at("192.168.1.10", "255.255.255.0")];
        let chosen = candidate(&parse("192.168.1.40, 192.168.1.20"), &mine);
        assert_eq!(chosen, Some("192.168.1.20".parse().expect("an address")));
    }

    /// One unreadable entry does not lose the rest: this is somebody
    /// else's list.
    #[test]
    fn a_list_with_rubbish_in_it_keeps_what_it_can() {
        assert_eq!(
            parse("192.168.1.20, not-an-address, ,192.168.1.21"),
            vec![
                "192.168.1.20".parse::<Ipv4Addr>().expect("one"),
                "192.168.1.21".parse::<Ipv4Addr>().expect("two"),
            ]
        );
    }

    /// What the kernel says about this machine, whatever machine that is.
    ///
    /// Runs on the node and on a laptop — the reason `getifaddrs` was
    /// chosen over `/proc`. It asserts the filters rather than any
    /// particular address, because there is no address a test can expect.
    #[test]
    fn what_this_machine_holds_is_worth_naming() {
        for one in held() {
            assert!(!one.address.is_loopback(), "{one:?}");
            assert!(!one.address.is_unspecified(), "{one:?}");
            assert!(!is_platform(one.address), "the platform's own: {one:?}");
            assert_ne!(one.mask, Ipv4Addr::UNSPECIFIED, "{one:?}");
        }
    }
}
