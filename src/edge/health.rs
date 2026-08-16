//! Which upstreams are answering.
//!
//! ## What this is for
//!
//! The edge picks upstreams by turn, one entry per replica, and until
//! now it never asked whether any of them answered. A dead replica kept
//! its share of the traffic until a person looked at a page — so a
//! service on three machines failed a third of its requests, silently,
//! and the multi-node feature added failure surface without adding
//! availability. See `docs/roadmap.md`.
//!
//! ## A TCP connect, and not more
//!
//! Whether something is listening on the address the edge would send to.
//! Not an HTTP request: a path to fetch and a status to expect are
//! per-service configuration this does not have, and inventing one — a
//! `GET /` that a service answers 404 to is not a sick service — would
//! make this wrong in a way nobody could see. A connect distinguishes
//! the case that matters, which is nothing there at all.
//!
//! ## It may take a replica out. It may never take a name down.
//!
//! **The rule that makes this safe to run.** If every upstream of a name
//! reads as down, the edge sends the request anyway rather than
//! answering as though the site did not exist. A wrong verdict here then
//! costs a 502 that would have happened regardless; it cannot cost a
//! name its route. The failure this rules out is the one worth ruling
//! out: a prober that is itself broken taking the whole console with it
//! at four in the morning.
//!
//! ## Slow to evict, fast to restore
//!
//! [`FAILURES_TO_EVICT`] consecutive failures before an address is taken
//! out; **one** success puts it back. The asymmetry is deliberate — a
//! blip must not move traffic, and a machine that has come back should
//! carry its share again on the next request rather than after a
//! quorum's worth of probes.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;

/// How many consecutive failed connects take an upstream out of the
/// rotation.
///
/// Two, at the interval the prober runs, so a single dropped packet is
/// not a deployment. One would make this jumpy on a busy node; five
/// would mean half a minute of sending traffic at nothing.
pub const FAILURES_TO_EVICT: u32 = 2;

/// How long to wait for a connect before calling it a failure.
///
/// Short: an upstream is a container on this machine's bridge or a node
/// across the overlay, and either answers a SYN in milliseconds or is
/// not going to. A generous timeout here is a prober that takes longer
/// than its own interval when several are down.
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(750);

/// What is known about one address.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Record {
    failures: u32,
    /// When it was first seen failing, so a page can say how long — and
    /// so "down" is a fact with a time on it rather than a flag.
    failing_since: Option<i64>,
}

impl Record {
    fn up(&self) -> bool {
        self.failures < FAILURES_TO_EVICT
    }
}

/// What the edge knows about its upstreams.
///
/// A `Mutex<HashMap>` rather than the `ArcSwap` the route table uses,
/// and the difference is the access pattern: routes are read on every
/// request and written almost never, while this is written by one prober
/// and read once per proxied request. A mutex held for a hash lookup is
/// cheaper than rebuilding the map on every probe.
#[derive(Debug, Default)]
pub struct Health {
    seen: Mutex<HashMap<SocketAddr, Record>>,
}

impl Health {
    /// Whether this address should take traffic.
    ///
    /// **An address nobody has probed is up.** That is the same rule as
    /// the paragraph about never taking a name down, one level lower: a
    /// replica deployed a second ago has no verdict, and starting it out
    /// as down would mean every deployment moved its own traffic away
    /// from itself until the next probe.
    pub fn is_up(&self, address: &SocketAddr) -> bool {
        self.seen
            .lock()
            .map(|seen| seen.get(address).is_none_or(|record| record.up()))
            .unwrap_or(true)
    }

    /// Record one probe.
    pub fn observed(&self, address: SocketAddr, answered: bool, at: i64) {
        let Ok(mut seen) = self.seen.lock() else {
            return;
        };
        let record = seen.entry(address).or_default();
        let was = record.up();
        match answered {
            // One success is enough — see the module docs.
            true => *record = Record::default(),
            false => {
                record.failures = record.failures.saturating_add(1);
                record.failing_since.get_or_insert(at);
            }
        }
        // Said once at each edge, not on every probe: a node with an
        // upstream down would otherwise write a line every few seconds
        // for as long as it stayed down, which is how a log stops being
        // read.
        match (was, record.up()) {
            (true, false) => {
                tracing::warn!(%address, "upstream stopped answering; out of rotation")
            }
            (false, true) => tracing::info!(%address, "upstream is answering again"),
            _ => {}
        }
    }

    /// Every address currently out of the rotation, and since when.
    ///
    /// For the page. An operator asking why a service is slow needs to
    /// see which copy is not taking its share, which is a thing this
    /// knows and nothing else does.
    pub fn down(&self) -> Vec<(SocketAddr, Option<i64>)> {
        let Ok(seen) = self.seen.lock() else {
            return Vec::new();
        };
        let mut down: Vec<(SocketAddr, Option<i64>)> = seen
            .iter()
            .filter(|(_, record)| !record.up())
            .map(|(address, record)| (*address, record.failing_since))
            .collect();
        down.sort_by_key(|(address, _)| *address);
        down
    }

    /// Forget everything that is no longer an upstream.
    ///
    /// Called with what the route table holds, after each sweep. Without
    /// it this grows by one entry per address that ever existed, and a
    /// node that redeploys often allocates a new container address every
    /// time — so the map would be a slow leak keyed on ancient
    /// containers, and `down()` would report replicas that no longer
    /// exist as though they were failing.
    pub fn retain(&self, live: &[SocketAddr]) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.retain(|address, _| live.contains(address));
        }
    }
}

/// How often every upstream is asked.
///
/// Five seconds, against a two-failure threshold, puts ten seconds
/// between a replica dying and the traffic moving off it. Shorter costs
/// a connect per upstream more often on a one-core node; longer is ten
/// seconds of somebody's requests going nowhere.
pub const EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// Ask every upstream once, in parallel, and record what they said.
///
/// Returns how many are out of the rotation afterwards, so the caller
/// can log a change rather than a heartbeat.
///
/// Concurrent rather than in turn: the timeout is the cost of one
/// unreachable address, and a node with six of them would otherwise
/// spend longer sweeping than the interval between sweeps — at which
/// point the health of the first is as old as the sweep is long.
pub async fn sweep(routes: &super::routes::RouteTable, at: i64) -> usize {
    let health = routes.health();
    let addresses = routes.upstreams();

    // Before the probes, not after: an address the table no longer holds
    // must not be reported as failing, and doing this first also means a
    // sweep of nothing still cleans up after a service that was deleted.
    health.retain(&addresses);

    let answers = futures_util::future::join_all(
        addresses
            .iter()
            .map(|address| async move { (*address, probe(*address).await) }),
    )
    .await;

    for (address, answered) in answers {
        health.observed(address, answered, at);
    }
    health.down().len()
}

/// Probe every upstream for as long as the node runs.
pub async fn watch(
    routes: std::sync::Arc<super::routes::RouteTable>,
    cancel: wabot::lifecycle::Cancel,
) -> anyhow::Result<()> {
    let mut reported = 0usize;
    loop {
        let down = sweep(&routes, crate::platform::now_ms()).await;
        // Only when the count moves. A node with one upstream down is a
        // node that would otherwise say so twelve times a minute for as
        // long as it stayed down, and each address already logs its own
        // edges in `observed`.
        if down != reported {
            match down {
                0 => tracing::info!("every upstream is answering"),
                n => tracing::warn!(down = n, "upstreams out of rotation"),
            }
            reported = down;
        }

        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(EVERY) => {}
        }
    }
}

/// Ask one address whether anything is listening.
pub async fn probe(address: SocketAddr) -> bool {
    matches!(
        tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::TcpStream::connect(address)).await,
        Ok(Ok(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(last: u8) -> SocketAddr {
        SocketAddr::from(([10, 42, 0, last], 8080))
    }

    /// Slow to evict, fast to restore — and unknown is up, so a replica
    /// deployed a second ago does not move its own traffic away from
    /// itself while it waits for a first verdict.
    #[test]
    fn one_failure_is_a_blip_and_one_success_is_a_recovery() {
        let health = Health::default();
        let one = address(1);

        assert!(health.is_up(&one), "nothing known is not the same as down");

        health.observed(one, false, 1_000);
        assert!(
            health.is_up(&one),
            "one failed connect must not move traffic"
        );

        health.observed(one, false, 1_100);
        assert!(!health.is_up(&one), "two in a row is a verdict");
        assert_eq!(
            health.down(),
            vec![(one, Some(1_000))],
            "and it says since when — the first failure, not the eviction"
        );

        health.observed(one, true, 1_200);
        assert!(health.is_up(&one), "one success is enough to come back");
        assert!(health.down().is_empty());
    }

    /// The whole path, against real sockets: a listener that is there
    /// stays in the rotation, one that is gone leaves it, and the sweep
    /// is what moves the verdict.
    ///
    /// A real port rather than a mocked probe, because what this has to
    /// get right is what a TCP connect does to an address nothing is
    /// bound to — and on that, an assertion about my own mock would only
    /// tell me what I already believed.
    #[tokio::test]
    async fn a_listener_that_goes_away_leaves_the_rotation() {
        let alive = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let up = alive.local_addr().expect("addr");

        // Bound and dropped: the port is nobody's now, which is what a
        // container that died looks like from here.
        let down = {
            let gone = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            gone.local_addr().expect("addr")
        };

        let routes = std::sync::Arc::new(super::super::routes::RouteTable::new());
        routes.replace([(
            "api.example".to_string(),
            super::super::routes::Upstream::Proxy(vec![up, down]),
        )]);

        // One sweep is a blip, not a verdict.
        assert_eq!(sweep(&routes, 1_000).await, 0);
        assert!(routes.health().is_up(&down));

        assert_eq!(sweep(&routes, 1_100).await, 1, "two in a row is");
        assert!(routes.health().is_up(&up), "and the live one is untouched");

        // So every request goes to the one that answers.
        let upstream = routes.resolve("api.example").expect("routed");
        for _ in 0..4 {
            assert_eq!(routes.next_upstream(&upstream), Some(up));
        }

        drop(alive);
        for at in [1_200, 1_300] {
            sweep(&routes, at).await;
        }
        // Both gone, and the name is still routed rather than dropped.
        assert_eq!(routes.health().down().len(), 2);
        assert!(
            routes.next_upstream(&upstream).is_some(),
            "a name whose upstreams all died still forwards, and 502s"
        );
    }

    /// An address that is no longer an upstream is forgotten. A node
    /// that redeploys gets a new container address each time, so without
    /// this the map grows for ever and `down()` reports replicas that no
    /// longer exist.
    #[test]
    fn what_is_no_longer_an_upstream_is_forgotten() {
        let health = Health::default();
        let (gone, kept) = (address(1), address(2));
        for _ in 0..FAILURES_TO_EVICT {
            health.observed(gone, false, 1_000);
            health.observed(kept, false, 1_000);
        }
        assert_eq!(health.down().len(), 2);

        health.retain(&[kept]);
        assert_eq!(health.down(), vec![(kept, Some(1_000))]);
        assert!(
            health.is_up(&gone),
            "and a forgotten one is unknown, which is up"
        );
    }
}
