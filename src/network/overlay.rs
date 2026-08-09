//! Addresses on the overlay, handed out by whoever is enrolling.
//!
//! Nothing routes yet — the tunnel is the next phase. What exists now
//! is the allocation, because the address travels in the join token and
//! so has to be decided before anything is dialled.
//!
//! ## Who allocates
//!
//! The node minting the token. That is the whole of the coordination:
//! addresses come from one place per authority, so there is no second
//! allocator to agree with. Two authorities enrolling the same node
//! would give it two addresses, which is correct — it is on two
//! overlays.
//!
//! ## Why this range
//!
//! `10.42.0.0/16` is RFC 1918 space, and the middle of it: `10.0.0.0/24`
//! and `10.1.0.0/16` are what everything else picks by default, and an
//! overlay that collides with the network the node is already on is a
//! machine that loses its own default route. Sixty-five thousand
//! addresses is more nodes than this product will ever put on one
//! overlay, and picking a /16 rather than a /24 means never having to
//! renumber.

use std::collections::BTreeSet;

use wabot::sqlite::{SqliteDatabase, SqliteResult};

use super::{NetworkError, NetworkResult};

/// The overlay's address space, as an operator would write it.
pub const SUBNET: &str = "10.42.0.0/16";

/// How much of an address is the network. The interface carries this
/// rather than a `/32`, which is what routes every other overlay
/// address through it.
pub const PREFIX_LENGTH: u8 = 16;

/// The first two octets, which every address here shares.
const PREFIX: (u8, u8) = (10, 42);

/// `10.42.0.0` is the network and `10.42.255.255` the broadcast, so
/// hosts run from 1 to 65534.
const FIRST: u32 = 1;
const LAST: u32 = 65_534;

/// The lowest address nothing is using.
///
/// Lowest rather than next, so an overlay whose nodes come and go stays
/// dense and readable: `10.42.0.3` is the third node, not the
/// three-hundredth thing that ever happened here.
///
/// Both tables are consulted. A pending enrolment holds its address —
/// it is already written into a token somebody is carrying — and
/// allocating it again would put two nodes on one address the moment
/// the second one joined.
pub async fn allocate(database: &SqliteDatabase) -> NetworkResult<String> {
    let taken = taken(database).await?;
    (FIRST..=LAST)
        .find(|host| !taken.contains(host))
        .map(address)
        // A refusal rather than a failure: it is the truthful answer to
        // what was asked, and it is the one an operator can act on.
        .ok_or_else(|| {
            NetworkError::Refused(format!(
                "the overlay {SUBNET} is full — all {LAST} addresses are in use"
            ))
        })
}

/// Every address this node has already promised or recorded.
async fn taken(database: &SqliteDatabase) -> SqliteResult<BTreeSet<u32>> {
    let addresses: Vec<String> = database
        .read(|connection| {
            let mut statement = connection.prepare(
                "SELECT \"overlay_ip\" FROM node WHERE \"overlay_ip\" IS NOT NULL \
                 UNION \
                 SELECT \"overlay_ip\" FROM enrolment",
            )?;
            let addresses: wabot::sqlite::rusqlite::Result<Vec<String>> = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect();
            addresses
        })
        .await?;

    // An address outside this range is not this allocator's to worry
    // about: it was written by hand or by an older range, and the only
    // thing that matters here is not handing it out again.
    Ok(addresses.iter().filter_map(|text| host(text)).collect())
}

fn address(host: u32) -> String {
    let (a, b) = PREFIX;
    format!("{a}.{b}.{}.{}", (host >> 8) & 0xff, host & 0xff)
}

/// The host part of an address in this range, if it is one.
fn host(text: &str) -> Option<u32> {
    let mut octets = text.trim().split('.');
    let quad: [u8; 4] = [
        octets.next()?.parse().ok()?,
        octets.next()?.parse().ok()?,
        octets.next()?.parse().ok()?,
        octets.next()?.parse().ok()?,
    ];
    if octets.next().is_some() || (quad[0], quad[1]) != PREFIX {
        return None;
    }
    Some((u32::from(quad[2]) << 8) | u32::from(quad[3]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{save, Kind, Node};

    async fn node_at(database: &SqliteDatabase, id: &str, overlay_ip: &str) {
        save(
            database,
            &Node {
                id: id.into(),
                name: id.into(),
                kind: Kind::Private,
                endpoint: None,
                public_key: None,
                overlay_ip: Some(overlay_ip.into()),
                is_self: false,
                last_seen_at: None,
            },
        )
        .await
        .expect("save");
    }

    #[tokio::test]
    async fn the_first_address_on_an_empty_overlay_is_the_first_one() {
        let database = crate::db::open_in_memory().await.expect("open");
        assert_eq!(allocate(&database).await.expect("allocate"), "10.42.0.1");
    }

    /// Lowest free, not next: an overlay whose nodes come and go stays
    /// readable rather than counting everything that ever happened.
    #[tokio::test]
    async fn a_freed_address_is_handed_out_again() {
        let database = crate::db::open_in_memory().await.expect("open");
        node_at(&database, "one", "10.42.0.1").await;
        node_at(&database, "three", "10.42.0.3").await;

        assert_eq!(allocate(&database).await.expect("allocate"), "10.42.0.2");
    }

    /// The address in a token somebody is carrying is spoken for. It
    /// used to be free until the node arrived, which is the window in
    /// which two nodes get one address.
    #[tokio::test]
    async fn a_pending_enrolment_holds_its_address() {
        let database = crate::db::open_in_memory().await.expect("open");
        let admin = crate::network::tests::admin(&database).await;

        let first = allocate(&database).await.expect("allocate");
        crate::network::enrolment::create(&database, "alpine", &first, &admin)
            .await
            .expect("minted");

        assert_ne!(allocate(&database).await.expect("allocate"), first);
    }

    /// Addresses roll over the third octet rather than stopping at 254.
    #[test]
    fn the_range_is_a_sixteen_and_reads_back() {
        assert_eq!(address(1), "10.42.0.1");
        assert_eq!(address(255), "10.42.0.255");
        assert_eq!(address(256), "10.42.1.0");
        assert_eq!(address(LAST), "10.42.255.254");

        for host in [FIRST, 255, 256, 4_242, LAST] {
            assert_eq!(super::host(&address(host)), Some(host));
        }
    }

    /// Something outside the range is somebody else's business — the
    /// only thing that matters is that it is not read as one of ours.
    #[test]
    fn an_address_from_somewhere_else_is_not_one_of_these() {
        for text in ["10.0.0.1", "192.168.1.4", "", "10.42.0", "10.42.0.1.2", "x"] {
            assert_eq!(super::host(text), None, "{text}");
        }
    }
}
