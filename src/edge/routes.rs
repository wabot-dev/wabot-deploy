//! Which hostname goes where.
//!
//! ## Read on every request, written almost never
//!
//! An `ArcSwap` of the whole map rather than a lock around it. Every
//! request resolves a host; a route changes when somebody deploys.
//! Putting the read path behind a lock would serialize the node's
//! traffic behind its rarest operation.
//!
//! ## The control plane is a row, not a special case
//!
//! "Which host serves the console" is then something an operator can
//! read out of the database and change, rather than a constant
//! compiled into the dispatch.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use arc_swap::ArcSwap;
use wabot::sqlite::{SqliteDatabase, SqliteResult};

/// Where a hostname's traffic goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Upstream {
    /// The node's own API and console. Served in-process — the router
    /// is a `tower::Service`, so there is no socket and no second
    /// serialization.
    ControlPlane,
    /// The replicas of one service, in the order they were placed.
    ///
    /// **One entry per replica, not per node.** A node running two
    /// copies contributes two, so a plain round-robin sends it twice
    /// the requests without anything having to carry a weight — which
    /// is what "two replicas there and one here" is asking for.
    ///
    /// Never empty: a name with nothing behind it is dropped from the
    /// table rather than kept pointing at nowhere, because a 404 from
    /// the edge is a truthful answer and a proxy attempt to a dead
    /// address is a hung request.
    Proxy(Vec<SocketAddr>),
}

impl Upstream {
    /// Which one takes this request.
    ///
    /// Round-robin, and the counter is per table rather than per name:
    /// two names with two upstreams each would otherwise start every
    /// request at the same index and send the whole first burst to the
    /// same pair of containers.
    ///
    /// Not least-connections or latency-aware. Those need a measurement
    /// per request, where this needs one signal per replica — see
    /// [`pick_live`](Self::pick_live), which is what the edge calls.
    pub fn pick(&self, turn: usize) -> Option<SocketAddr> {
        match self {
            Self::ControlPlane => None,
            Self::Proxy(addresses) if addresses.is_empty() => None,
            Self::Proxy(addresses) => Some(addresses[turn % addresses.len()]),
        }
    }

    /// Which one takes this request, skipping the ones that are not
    /// answering.
    ///
    /// Round-robin **over the live ones**, which is not the same as
    /// round-robin skipping the dead ones — and the difference is the
    /// whole load-balancing model.
    ///
    /// The first version scanned the full ring from `turn` and took the
    /// first live entry. With three replicas and the middle one down
    /// that sends four requests in six to the *third* and two to the
    /// first: the dead one's share goes entirely to whoever follows it
    /// rather than being spread. A test caught it, and it matters
    /// because the promise here is "one entry per replica, so a node
    /// with two copies takes twice the traffic" — a promise that must
    /// not quietly change shape at the moment a machine dies, which is
    /// when somebody is reading the graph.
    ///
    /// So: count the live ones, then take the `turn`th of those. Even
    /// across whatever is left, and unchanged while everything is up.
    ///
    /// **If nothing is answering it sends the request anyway.** A name
    /// whose every upstream reads as down still gets a request
    /// forwarded, which fails with a 502 that would have happened
    /// regardless — rather than a 404, which claims the site does not
    /// exist. This is what keeps a broken prober from being able to take
    /// a name off the air: health may remove a replica *while another
    /// one is up*, and it may never remove the last.
    pub fn pick_live(&self, turn: usize, health: &super::health::Health) -> Option<SocketAddr> {
        let Self::Proxy(addresses) = self else {
            return None;
        };
        let live = addresses
            .iter()
            .filter(|address| health.is_up(address))
            .count();
        if live == 0 {
            // Which also answers the empty case, with `None`.
            return self.pick(turn);
        }
        addresses
            .iter()
            .filter(|address| health.is_up(address))
            .nth(turn % live)
            .copied()
    }
}

/// Hostname → upstream, replaced whole.
pub struct RouteTable {
    by_host: ArcSwap<HashMap<String, Upstream>>,
    /// Where the next request starts looking. Shared across names on
    /// purpose — see [`Upstream::pick`].
    turn: std::sync::atomic::AtomicUsize,
    /// Which upstreams are answering.
    ///
    /// Beside the table rather than in it, because the two are different
    /// facts with different lifetimes: the table says where a name *may*
    /// go and changes when somebody deploys, this says which of those
    /// are answering *now* and changes on a timer. Folding health into
    /// the table would mean rebuilding it every few seconds and would
    /// make a name with nothing healthy behind it disappear — which is
    /// exactly the answer `pick_live` refuses to give.
    health: Arc<super::health::Health>,
}

impl Default for RouteTable {
    fn default() -> Self {
        Self::new()
    }
}

impl RouteTable {
    pub fn new() -> Self {
        Self {
            by_host: ArcSwap::from_pointee(HashMap::new()),
            turn: std::sync::atomic::AtomicUsize::new(0),
            health: Arc::new(super::health::Health::default()),
        }
    }

    /// What the prober writes into and the console reads out of.
    pub fn health(&self) -> Arc<super::health::Health> {
        self.health.clone()
    }

    /// Every address behind every name, for the prober to ask about.
    ///
    /// Deduplicated: one node's overlay address serves every name it
    /// answers for, and probing it once per name would be the same
    /// question asked five times.
    pub fn upstreams(&self) -> Vec<SocketAddr> {
        let mut all: Vec<SocketAddr> = self
            .by_host
            .load()
            .values()
            .filter_map(|upstream| match upstream {
                Upstream::Proxy(addresses) => Some(addresses.clone()),
                Upstream::ControlPlane => None,
            })
            .flatten()
            .collect();
        all.sort();
        all.dedup();
        all
    }

    /// Resolve a `Host` header.
    ///
    /// The port is stripped first: `example.com` and `example.com:8443`
    /// are the same site, and a client is free to include the port
    /// even when it is the default one.
    /// The next replica to take a request for this upstream.
    ///
    /// The counter is here rather than on the route because a route is
    /// replaced whole on every change — a per-route counter would reset
    /// to zero every time a deployment finished, and the first burst
    /// after each one would all land on the same container.
    pub fn next_upstream(&self, upstream: &Upstream) -> Option<SocketAddr> {
        let turn = self.turn.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        upstream.pick_live(turn, &self.health)
    }

    pub fn resolve(&self, host: &str) -> Option<Upstream> {
        self.by_host.load().get(&normalize(host)).cloned()
    }

    pub fn replace(&self, routes: impl IntoIterator<Item = (String, Upstream)>) {
        let map: HashMap<String, Upstream> = routes
            .into_iter()
            .map(|(host, upstream)| (normalize(&host), upstream))
            .collect();
        self.by_host.store(Arc::new(map));
    }

    pub fn hosts(&self) -> Vec<String> {
        let mut hosts: Vec<String> = self.by_host.load().keys().cloned().collect();
        hosts.sort();
        hosts
    }

    pub fn is_empty(&self) -> bool {
        self.by_host.load().is_empty()
    }
}

/// Lowercase, no port, no trailing dot.
///
/// All three are things a client legitimately sends and none of them
/// change which site is meant. A table keyed on the raw header would
/// miss `EXAMPLE.com:443` for a route stored as `example.com`.
fn normalize(host: &str) -> String {
    let host = host.trim();
    // An IPv6 literal is bracketed, and the colons inside it are not a
    // port separator — `[::1]:8443` must not become `[`.
    let without_port = if let Some(end) = host.strip_prefix('[').and_then(|_| host.find(']')) {
        let (literal, rest) = host.split_at(end + 1);
        if rest.starts_with(':') {
            literal
        } else {
            host
        }
    } else {
        match host.rsplit_once(':') {
            Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name,
            _ => host,
        }
    };
    without_port.trim_end_matches('.').to_ascii_lowercase()
}

// ---------- storage ---------------------------------------------------

/// Drop every proxy route not in `keep`.
///
/// Control-plane rows are never touched: they are what makes the
/// console reachable, and a sync that removed them because no service
/// claimed the node's own domain would lock the operator out of the
/// thing they would fix it from.
pub async fn retain_proxies(database: &SqliteDatabase, keep: &[String]) -> SqliteResult<usize> {
    let keep: Vec<String> = keep.iter().map(|host| normalize(host)).collect();
    database
        .write(move |connection| {
            let mut removed = 0;
            // Only routes this node's own services own. A proxy route
            // with no local service behind it was written by an errand
            // — this node is somebody else's edge for that name — and
            // pruning it here would take the name off the air every
            // time anything deployed locally, for a reason nobody
            // looking at either node could see.
            let hosts: Vec<String> = connection
                .prepare(
                    "SELECT \"host\" FROM route \
                     WHERE \"upstream_kind\" = 'proxy' AND \"service_id\" IS NOT NULL",
                )?
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;

            for host in hosts {
                if !keep.contains(&host) {
                    connection.execute("DELETE FROM route WHERE \"host\" = ?1", [&host])?;
                    removed += 1;
                }
            }
            Ok(removed)
        })
        .await
}

/// Forget a name this node was answering **for another node**.
///
/// The gap between the two functions above, and nothing filled it:
/// `retain_proxies` skips a row with no `service_id` on purpose, and
/// `forget_control_plane` touches only control-plane rows. A route
/// written by an edge errand is a proxy row with no service, so it was
/// in neither set — the owner could take the name back and this node
/// would go on proxying it for ever.
///
/// Scoped to exactly that shape: never a local service's route, never
/// the console's.
pub async fn forget_for_other(database: &SqliteDatabase, host: &str) -> SqliteResult<usize> {
    let host = normalize(host);
    database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM route \
                 WHERE \"host\" = ?1 AND \"upstream_kind\" = 'proxy' \
                   AND \"service_id\" IS NULL",
                [host],
            )
        })
        .await
}

/// Forget one control-plane name.
///
/// For the node that was renamed: the old name keeps answering the
/// console otherwise, which is a name the operator stopped meaning to
/// serve and stopped having a certificate for.
///
/// Proxy rows are left alone — those belong to `retain_proxies`, which
/// derives them from the services, and to [`forget_for_other`].
pub async fn forget_control_plane(database: &SqliteDatabase, host: &str) -> SqliteResult<()> {
    let host = normalize(host);
    // Never the fallback: `localhost` is how somebody reaches the
    // console from the node itself when everything else is wrong.
    if host == crate::edge::certs::FALLBACK_NAME {
        return Ok(());
    }
    database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM route WHERE \"host\" = ?1 AND \"upstream_kind\" = 'control_plane'",
                [host],
            )?;
            Ok(())
        })
        .await
}

/// Load every enabled route.
///
/// A row whose upstream cannot be parsed is skipped with a warning
/// rather than failing the load: one bad route must not take the whole
/// edge down, including the console you would fix it from.
pub async fn load_all(database: &SqliteDatabase) -> SqliteResult<Vec<(String, Upstream)>> {
    let rows: Vec<(String, String, Option<String>)> = database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"host\", \"upstream_kind\", \"upstream_addr\" \
                     FROM route WHERE \"enabled\" = 1 ORDER BY \"host\"",
                )?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .collect()
        })
        .await?;

    Ok(rows
        .into_iter()
        .filter_map(|(host, kind, addr)| match kind.as_str() {
            "control_plane" => Some((host, Upstream::ControlPlane)),
            // Comma-separated, because a route is now *n* replicas.
            // One that will not parse is dropped and the rest kept: a
            // service with three copies and one bad row should serve
            // from the two that are fine, which is the whole reason
            // there are three.
            "proxy" => {
                let addresses: Vec<SocketAddr> = addr
                    .as_deref()
                    .unwrap_or_default()
                    .split(',')
                    .filter(|part| !part.is_empty())
                    .filter_map(|part| match part.parse() {
                        Ok(address) => Some(address),
                        Err(_) => {
                            tracing::warn!(%host, part, "upstream skipped: not an address");
                            None
                        }
                    })
                    .collect();
                match addresses.is_empty() {
                    true => {
                        tracing::warn!(%host, "route skipped: nothing to proxy to");
                        None
                    }
                    false => Some((host, Upstream::Proxy(addresses))),
                }
            }
            other => {
                tracing::warn!(%host, kind = other, "route skipped: unknown upstream kind");
                None
            }
        })
        .collect())
}

/// Write a route, replacing whatever held that hostname.
///
/// Keyed on the hostname rather than the service: a name points at one
/// upstream, and a service that took over a name should take it over
/// rather than add a second row that the load order decides between.
pub async fn upsert(
    database: &SqliteDatabase,
    host: &str,
    upstream: &Upstream,
    service_id: Option<&str>,
) -> SqliteResult<()> {
    let host = normalize(host);
    let (kind, addr) = match upstream {
        Upstream::ControlPlane => ("control_plane".to_string(), None),
        Upstream::Proxy(addresses) => (
            "proxy".to_string(),
            Some(
                addresses
                    .iter()
                    .map(|address| address.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
        ),
    };
    let service_id = service_id.map(str::to_string);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default();

    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO route \
                   (\"host\", \"upstream_kind\", \"upstream_addr\", \"service_id\", \
                    \"enabled\", \"updated_at\") \
                 VALUES (?1, ?2, ?3, ?4, 1, ?5) \
                 ON CONFLICT (\"host\") DO UPDATE SET \
                   \"upstream_kind\" = excluded.\"upstream_kind\", \
                   \"upstream_addr\" = excluded.\"upstream_addr\", \
                   \"service_id\" = excluded.\"service_id\", \
                   \"enabled\" = 1, \
                   \"updated_at\" = excluded.\"updated_at\"",
                (host, kind, addr, service_id, now),
            )?;
            Ok(())
        })
        .await
}

#[cfg(test)]
mod tests {
    /// Health takes a replica out of the ring and leaves the ring
    /// turning. The share each live replica carries is unchanged —
    /// a node running two copies still appears twice — because health
    /// only removes entries, it does not reweight the ones that stay.
    #[test]
    fn a_replica_that_stopped_answering_is_skipped() {
        let table = RouteTable::new();
        let health = table.health();
        let (one, two, three) = (address(1), address(2), address(3));
        let upstream = Upstream::Proxy(vec![one, two, three]);

        // Everything up: three requests, three different copies.
        let taken: Vec<_> = (0..3)
            .map(|turn| upstream.pick_live(turn, &health))
            .collect();
        assert_eq!(taken, vec![Some(one), Some(two), Some(three)]);

        for _ in 0..crate::edge::health::FAILURES_TO_EVICT {
            health.observed(two, false, 0);
        }

        // Six requests over two live copies, evenly, and never the one
        // that is down.
        let taken: Vec<_> = (0..6)
            .map(|turn| upstream.pick_live(turn, &health).expect("somewhere"))
            .collect();
        assert!(!taken.contains(&two), "{taken:?}");
        assert_eq!(taken.iter().filter(|a| **a == one).count(), 3, "{taken:?}");
        assert_eq!(
            taken.iter().filter(|a| **a == three).count(),
            3,
            "{taken:?}"
        );
    }

    /// **The rule that makes this safe to run at all.** A name whose
    /// every upstream reads as down still gets a request forwarded: it
    /// fails with a 502 that would have happened anyway, rather than a
    /// 404 claiming the site does not exist.
    ///
    /// So a prober that is itself wrong — a node whose own network broke,
    /// a timeout too short under load — costs nothing that was not
    /// already lost. It can never take a name off the air, which is the
    /// failure that would make health checks worse than none.
    #[test]
    fn health_may_never_take_the_last_one_out() {
        let table = RouteTable::new();
        let health = table.health();
        let (one, two) = (address(1), address(2));
        let upstream = Upstream::Proxy(vec![one, two]);

        for address in [one, two] {
            for _ in 0..crate::edge::health::FAILURES_TO_EVICT {
                health.observed(address, false, 0);
            }
        }

        assert_eq!(
            upstream.pick_live(0, &health),
            Some(one),
            "with nothing healthy it still sends the request somewhere"
        );
        assert_eq!(
            upstream.pick_live(1, &health),
            Some(two),
            "and still in turn"
        );
    }

    /// The prober asks the table what to probe, and asks about one
    /// address once — a node's overlay address serves every name it
    /// answers for, so probing per name would be the same question five
    /// times.
    #[test]
    fn every_upstream_is_named_once_whatever_it_serves() {
        let table = RouteTable::new();
        let (shared, only) = (address(1), address(2));
        table.replace([
            ("a.example".to_string(), Upstream::Proxy(vec![shared, only])),
            ("b.example".to_string(), Upstream::Proxy(vec![shared])),
            ("console.example".to_string(), Upstream::ControlPlane),
        ]);

        assert_eq!(
            table.upstreams(),
            vec![shared, only],
            "deduplicated, and the control plane is not an upstream"
        );
    }

    fn address(last: u8) -> SocketAddr {
        SocketAddr::from(([10, 42, 0, last], 8080))
    }

    use super::*;

    fn proxy(port: u16) -> Upstream {
        Upstream::Proxy(vec![SocketAddr::from(([127, 0, 0, 1], port))])
    }

    /// Everything a client can legitimately vary without meaning a
    /// different site.
    /// One entry per replica, not per node — so a node running two
    /// copies takes twice the requests without anything carrying a
    /// weight. That is the whole of "two there and one here".
    #[test]
    fn requests_go_round_the_replicas_in_turn() {
        let table = RouteTable::new();
        let upstream = Upstream::Proxy(vec![
            "10.42.0.3:20001".parse().unwrap(), // two on one node
            "10.42.0.3:20002".parse().unwrap(),
            "10.42.0.1:20001".parse().unwrap(), // one on another
        ]);

        let mut seen = std::collections::BTreeMap::new();
        for _ in 0..30 {
            let picked = table.next_upstream(&upstream).expect("an upstream");
            *seen.entry(picked.to_string()).or_insert(0) += 1;
        }

        assert_eq!(seen["10.42.0.3:20001"], 10);
        assert_eq!(seen["10.42.0.3:20002"], 10);
        assert_eq!(seen["10.42.0.1:20001"], 10);
        // Which is twice the traffic for the node holding two of them.
        assert_eq!(seen["10.42.0.3:20001"] + seen["10.42.0.3:20002"], 20);
    }

    /// The counter lives on the table, not the route: a route is
    /// replaced whole on every change, so a counter inside one would
    /// reset every time a deployment finished and send the first burst
    /// after each to the same container.
    #[test]
    fn replacing_a_route_does_not_send_everybody_back_to_the_first() {
        let table = RouteTable::new();
        let upstream = Upstream::Proxy(vec![
            "10.42.0.1:1".parse().unwrap(),
            "10.42.0.1:2".parse().unwrap(),
        ]);

        let first = table.next_upstream(&upstream).expect("one");
        table.replace([("app.example".to_string(), upstream.clone())]);
        let after = table.next_upstream(&upstream).expect("one");

        assert_ne!(first, after, "the turn restarted");
    }

    /// A name with nothing behind it is not a route. The edge answering
    /// 404 is truthful; proxying to a dead address is a hung request.
    #[test]
    fn an_upstream_with_no_replicas_picks_nothing() {
        let table = RouteTable::new();
        assert_eq!(table.next_upstream(&Upstream::Proxy(Vec::new())), None);
        assert_eq!(table.next_upstream(&Upstream::ControlPlane), None);
    }

    #[test]
    fn a_host_header_is_normalized_before_lookup() {
        let table = RouteTable::new();
        table.replace([("Example.COM".to_string(), Upstream::ControlPlane)]);

        for header in [
            "example.com",
            "Example.com",
            "EXAMPLE.COM",
            "example.com:443",
            "example.com:8443",
            "example.com.",
            "  example.com  ",
        ] {
            assert_eq!(
                table.resolve(header),
                Some(Upstream::ControlPlane),
                "{header:?} should resolve"
            );
        }
        assert_eq!(table.resolve("other.com"), None);
    }

    /// An IPv6 literal is full of colons and none of them are the port
    /// separator — splitting on the last colon unconditionally would
    /// mangle it.
    #[test]
    fn an_ipv6_literal_survives_normalization() {
        assert_eq!(normalize("[::1]"), "[::1]");
        assert_eq!(normalize("[::1]:8443"), "[::1]");
        assert_eq!(normalize("[2001:DB8::1]:443"), "[2001:db8::1]");
    }

    /// A hostname can contain a colon only as a port separator, but a
    /// malformed one must not silently truncate the name.
    #[test]
    fn a_non_numeric_suffix_is_not_a_port() {
        assert_eq!(normalize("example.com:notaport"), "example.com:notaport");
    }

    #[tokio::test]
    async fn routes_round_trip_through_the_database() {
        let database = crate::db::open_in_memory().await.expect("open");

        upsert(&database, "node.example.com", &Upstream::ControlPlane, None)
            .await
            .expect("upsert");
        upsert(&database, "app.example.com", &proxy(8080), Some("svc-1"))
            .await
            .expect("upsert");

        let table = RouteTable::new();
        table.replace(load_all(&database).await.expect("load"));

        assert_eq!(table.hosts().len(), 2);
        assert_eq!(
            table.resolve("node.example.com"),
            Some(Upstream::ControlPlane)
        );
        assert_eq!(table.resolve("app.example.com"), Some(proxy(8080)));
    }

    #[tokio::test]
    async fn upserting_a_host_replaces_its_upstream() {
        let database = crate::db::open_in_memory().await.expect("open");

        upsert(&database, "app.example.com", &proxy(8080), None)
            .await
            .expect("upsert");
        upsert(&database, "app.example.com", &proxy(9090), None)
            .await
            .expect("upsert");

        let routes = load_all(&database).await.expect("load");
        assert_eq!(routes.len(), 1, "one row per host");
        assert_eq!(routes[0].1, proxy(9090));
    }

    /// One unparseable row must not take the edge down with it — the
    /// console you would fix it from is behind the same table.
    #[tokio::test]
    async fn a_broken_row_is_skipped_not_fatal() {
        let database = crate::db::open_in_memory().await.expect("open");
        upsert(&database, "good.example.com", &Upstream::ControlPlane, None)
            .await
            .expect("upsert");

        database
            .write(|connection| {
                connection.execute(
                    "INSERT INTO route (\"host\", \"upstream_kind\", \"upstream_addr\", \
                       \"enabled\", \"updated_at\") \
                     VALUES ('bad.example.com', 'proxy', 'not-an-address', 1, 0)",
                    [],
                )?;
                connection.execute(
                    "INSERT INTO route (\"host\", \"upstream_kind\", \"enabled\", \"updated_at\") \
                     VALUES ('weird.example.com', 'teleport', 1, 0)",
                    [],
                )?;
                Ok(())
            })
            .await
            .expect("insert");

        let routes = load_all(&database).await.expect("load");
        assert_eq!(routes.len(), 1, "the good route survived: {routes:?}");
        assert_eq!(routes[0].0, "good.example.com");
    }

    #[tokio::test]
    async fn a_disabled_route_is_not_loaded() {
        let database = crate::db::open_in_memory().await.expect("open");
        upsert(&database, "off.example.com", &proxy(8080), None)
            .await
            .expect("upsert");
        database
            .write(|connection| {
                connection.execute("UPDATE route SET \"enabled\" = 0", [])?;
                Ok(())
            })
            .await
            .expect("disable");

        assert!(load_all(&database).await.expect("load").is_empty());
    }
}
