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
    /// A container on this node.
    Proxy(SocketAddr),
}

/// Hostname → upstream, replaced whole.
pub struct RouteTable {
    by_host: ArcSwap<HashMap<String, Upstream>>,
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
        }
    }

    /// Resolve a `Host` header.
    ///
    /// The port is stripped first: `example.com` and `example.com:8443`
    /// are the same site, and a client is free to include the port
    /// even when it is the default one.
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
            "proxy" => match addr.as_deref().and_then(|a| a.parse().ok()) {
                Some(addr) => Some((host, Upstream::Proxy(addr))),
                None => {
                    tracing::warn!(
                        %host,
                        addr = addr.unwrap_or_default(),
                        "route skipped: unparseable upstream address"
                    );
                    None
                }
            },
            other => {
                tracing::warn!(%host, kind = other, "route skipped: unknown upstream kind");
                None
            }
        })
        .collect())
}

/// Nothing writes routes yet — deployments will, in M3. Until then
/// the table is populated only by tests, which insert directly, and a
/// writer with no caller would be an API designed against a guess.
#[cfg(test)]
pub async fn upsert(
    database: &SqliteDatabase,
    host: &str,
    upstream: &Upstream,
    service_id: Option<&str>,
) -> SqliteResult<()> {
    let host = normalize(host);
    let (kind, addr) = match upstream {
        Upstream::ControlPlane => ("control_plane".to_string(), None),
        Upstream::Proxy(addr) => ("proxy".to_string(), Some(addr.to_string())),
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
    use super::*;

    fn proxy(port: u16) -> Upstream {
        Upstream::Proxy(SocketAddr::from(([127, 0, 0, 1], port)))
    }

    /// Everything a client can legitimately vary without meaning a
    /// different site.
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
