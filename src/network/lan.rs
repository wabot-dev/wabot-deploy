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
