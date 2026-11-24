//! The port a node's WireGuard listens on, as part of where that node is.
//!
//! Every dialler used to assume the port it dialled was its own
//! `overlay.port`. That held while nothing moved it, and it was already
//! wrong for two nodes configured differently: each dialled the other at the
//! wrong port and they never met. Phase 13 moves it on purpose — a node
//! behind NAT whose tunnel has died takes a new port, because a router can
//! hold stale state on one pair of addresses and ports and pass every other
//! pair (`docs/network.md`, phase 13). So the port travels: each node says
//! its own on every report, the census relays it, and `tunnel::peers` dials
//! a peer at the port that peer said.
//!
//! Its own column, outside `Node`, for the reason `lan` is: `network::save`
//! writes the whole row and `ensure_self` calls it, so nothing rebuilt from a
//! report may reach it.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::{SqliteDatabase, SqliteResult};

use crate::infra::config::Config;

/// Where a new port is chosen from: the range IANA leaves for dynamic use.
///
/// Not a short band beside 51820. Nothing on a node guards UDP — its own
/// firewall chain is SSH and published ports — and a band small enough for
/// somebody to name in a rule is one a router can hold stale state across
/// as well.
pub const MOVES_WITHIN: RangeInclusive<u16> = 49152..=65535;

/// The port this node listens on: the one it moved to, or its configured
/// one if it never has.
///
/// **Never back to the configured one by itself.** The moved port is part
/// of this node's address now, and going back is one more move that can
/// meet the same router.
pub async fn own(database: &SqliteDatabase, config: &Config) -> u16 {
    let moved: SqliteResult<Option<Option<i64>>> = database
        .read(|connection| {
            connection
                .query_row(
                    "SELECT \"listen_port\" FROM node WHERE \"is_self\" = 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
        })
        .await;
    moved
        .ok()
        .flatten()
        .flatten()
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .unwrap_or(config.overlay.port)
}

/// Move this node to `port`, which the next `tunnel::ensure` puts on the
/// interface and the next report tells the authority.
pub async fn move_to(database: &SqliteDatabase, port: u16) -> SqliteResult<()> {
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE node SET \"listen_port\" = ?1 WHERE \"is_self\" = 1",
                [i64::from(port)],
            )?;
            Ok(())
        })
        .await
}

/// Store the port another node said it listens on.
///
/// `true` when it differs from what was stored. The caller runs
/// `tunnel::ensure` for that, because a sibling dialled at a port it has
/// left is a tunnel that never comes back; and only for that, because a
/// census arrives every fifteen seconds and almost always repeats itself.
pub async fn remember(database: &SqliteDatabase, node_id: &str, port: u16) -> SqliteResult<bool> {
    let node_id = node_id.to_string();
    database
        .write(move |connection| {
            let changed = connection.execute(
                "UPDATE node SET \"listen_port\" = ?2 \
                 WHERE \"id\" = ?1 AND \"is_self\" = 0 \
                 AND (\"listen_port\" IS NULL OR \"listen_port\" != ?2)",
                (node_id, i64::from(port)),
            )?;
            Ok(changed > 0)
        })
        .await
}

/// What every other node said its port is, by node id. A node missing from
/// the map has never said, which is a node older than the field.
pub async fn of_every_node(database: &SqliteDatabase) -> SqliteResult<BTreeMap<String, u16>> {
    database
        .read(|connection| {
            let mut statement = connection.prepare(
                "SELECT \"id\", \"listen_port\" FROM node \
                 WHERE \"is_self\" = 0 AND \"listen_port\" IS NOT NULL",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            let mut said = BTreeMap::new();
            for row in rows {
                let (id, port) = row?;
                if let Ok(port) = u16::try_from(port) {
                    said.insert(id, port);
                }
            }
            Ok(said)
        })
        .await
}

/// A port from [`MOVES_WITHIN`] that is not `current`.
///
/// Pure, with the randomness handed in, so a test can say what it does at
/// the edges of the range.
pub fn another(current: u16, entropy: u64) -> u16 {
    let (low, high) = (*MOVES_WITHIN.start(), *MOVES_WITHIN.end());
    let span = u64::from(high - low) + 1;
    let port = low + (entropy % span) as u16;
    match port == current {
        true if port == high => low,
        true => port + 1,
        false => port,
    }
}

/// Enough randomness to pick a port, without a dependency for it.
///
/// `RandomState` is seeded from the operating system once per process and
/// stepped for every new one, so two moves in one process differ too.
pub fn entropy() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default(),
    );
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new port is a new pair at the router only if it is a different
    /// port. Landing on the one it is leaving would be a move that changes
    /// nothing, five minutes before the next chance.
    #[test]
    fn a_new_port_is_never_the_one_it_leaves() {
        let (low, high) = (*MOVES_WITHIN.start(), *MOVES_WITHIN.end());
        let span = u64::from(high - low) + 1;

        assert_eq!(another(low, 0), low + 1);
        assert_eq!(another(high, span - 1), low);
        for entropy in [0, 1, 7, 4095, span - 1, span, u64::MAX] {
            for current in [low, low + 1, 51820, high - 1, high] {
                let port = another(current, entropy);
                assert_ne!(port, current);
                assert!(MOVES_WITHIN.contains(&port), "{port} is outside the range");
            }
        }
    }

    /// The configured port until it moves, and the moved one after —
    /// including after a restart, which is what keeps `needed` from
    /// rebuilding the interface back onto the port the router blocked.
    #[tokio::test]
    async fn a_node_listens_where_it_moved_to() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let config = Config::default();
        crate::infra::network::ensure_self(&database, &config)
            .await
            .expect("self");

        assert_eq!(own(&database, &config).await, config.overlay.port);
        move_to(&database, 53122).await.expect("move");
        assert_eq!(own(&database, &config).await, 53122);

        crate::infra::network::ensure_self(&database, &config)
            .await
            .expect("self again");
        assert_eq!(own(&database, &config).await, 53122);
    }

    /// Only a change is news: the census repeats itself every fifteen
    /// seconds, and each `true` is a netlink read and a line in the log.
    #[tokio::test]
    async fn a_port_said_again_is_not_a_change() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::network::learn_peer(
            &database,
            "nd-sibling",
            "sibling",
            "10.42.0.7",
            "tsrC7XtGKfhAE6bkgIPTGZwwPH3RwzSLwokRpc4JCRA=",
            "",
        )
        .await
        .expect("a sibling");

        assert!(remember(&database, "nd-sibling", 51820)
            .await
            .expect("first"));
        assert!(!remember(&database, "nd-sibling", 51820)
            .await
            .expect("again"));
        assert!(remember(&database, "nd-sibling", 53122)
            .await
            .expect("moved"));
        assert_eq!(
            of_every_node(&database)
                .await
                .expect("read")
                .get("nd-sibling"),
            Some(&53122)
        );
    }
}
