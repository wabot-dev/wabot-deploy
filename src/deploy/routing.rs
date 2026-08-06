//! Which hostname reaches which container.
//!
//! Derived, never edited: the routes are a function of the ports, the
//! services and their addresses. Anything else drifts — a service
//! redeployed onto a new address leaves a route pointing at the old
//! one, and the failure is a page that loads for nobody with nothing
//! in any log saying why.
//!
//! So the whole set is recomputed after every deployment. It is a
//! handful of rows; the alternative is bookkeeping that has to be
//! right in six places instead of one.

use std::net::SocketAddr;
use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::edge::routes::{self, RouteTable, Upstream};
use crate::platform::{ports, services, PlatformResult};

/// Recompute every route and hand the new set to the edge.
///
/// `table` is the live one the listener reads. Passing it in rather
/// than reaching for a global is what makes this callable from a test
/// with a table of its own.
pub async fn sync(
    database: &SqliteDatabase,
    node_domain: Option<&str>,
    table: Option<&Arc<RouteTable>>,
) -> PlatformResult<usize> {
    let ports = ports::all(database).await?;
    let services = services::all(database, None).await?;

    let mut hosts = Vec::new();
    for port in &ports {
        let Some(hostname) = &port.hostname else {
            continue;
        };
        let Some(service) = services.iter().find(|s| s.id == port.service_id) else {
            continue;
        };
        // No address means the container is not running. The route is
        // dropped rather than kept pointing at where it used to be: a
        // 404 from the edge is a truthful answer, and a proxy attempt
        // to a dead address is a hung request.
        let Some(address) = &service.address else {
            continue;
        };

        let Ok(upstream) = format!("{address}:{}", port.container_port).parse::<SocketAddr>()
        else {
            tracing::warn!(%address, port = port.container_port, "unroutable address");
            continue;
        };

        routes::upsert(
            database,
            hostname,
            &Upstream::Proxy(upstream),
            Some(&service.id),
        )
        .await?;
        hosts.push(hostname.clone());
    }

    routes::retain_proxies(database, &hosts).await?;

    // The console keeps its own names whatever the services do. The
    // edge only falls back to the control plane while *no* route
    // exists, so the first service hostname would otherwise take the
    // console off the air — on the node's own domain, which is where
    // somebody would go to undo it.
    for name in node_domain
        .into_iter()
        .chain(Some(crate::edge::certs::FALLBACK_NAME))
    {
        routes::upsert(database, name, &Upstream::ControlPlane, None).await?;
    }

    if let Some(table) = table {
        table.replace(routes::load_all(database).await?);
        tracing::info!(hosts = table.hosts().join(", "), "routes reloaded");
    }
    Ok(hosts.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::projects;

    async fn node() -> (SqliteDatabase, String, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "demo").await.expect("project");
        let service = services::create(
            &database,
            &project.id,
            "web",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("service");
        (database, project.id, service.id)
    }

    #[tokio::test]
    async fn a_running_service_with_a_hostname_gets_a_route() {
        let (database, _, service) = node().await;
        ports::create(&database, &service, 80, false, Some("api.example.com"))
            .await
            .expect("port");
        services::set_address(&database, &service, Some("10.42.1.5"))
            .await
            .expect("address");

        let table = Arc::new(RouteTable::new());
        assert_eq!(
            sync(&database, Some("node.example"), Some(&table))
                .await
                .expect("sync"),
            1
        );

        assert_eq!(
            table.resolve("api.example.com"),
            Some(Upstream::Proxy("10.42.1.5:80".parse().unwrap()))
        );
    }

    /// A route to a container that is not running is a request that
    /// hangs instead of a page that says no.
    #[tokio::test]
    async fn a_stopped_service_has_no_route() {
        let (database, _, service) = node().await;
        ports::create(&database, &service, 80, false, Some("api.example.com"))
            .await
            .expect("port");
        services::set_address(&database, &service, Some("10.42.1.5"))
            .await
            .expect("address");

        let table = Arc::new(RouteTable::new());
        sync(&database, Some("node.example"), Some(&table))
            .await
            .expect("sync");

        // Stopped: the address goes, and so must the route.
        services::set_address(&database, &service, None)
            .await
            .expect("address");
        sync(&database, Some("node.example"), Some(&table))
            .await
            .expect("sync");

        assert_eq!(table.resolve("api.example.com"), None);
    }

    /// The trap this exists to avoid: the edge serves the control
    /// plane on every hostname only while the route table is empty, so
    /// the first service route would take the console off the air.
    #[tokio::test]
    async fn the_console_keeps_its_own_hostname() {
        let (database, _, service) = node().await;
        ports::create(&database, &service, 80, false, Some("api.example.com"))
            .await
            .expect("port");
        services::set_address(&database, &service, Some("10.42.1.5"))
            .await
            .expect("address");

        let table = Arc::new(RouteTable::new());
        sync(&database, Some("node.example"), Some(&table))
            .await
            .expect("sync");

        assert_eq!(
            table.resolve("node.example"),
            Some(Upstream::ControlPlane),
            "the console is still reachable on the node's domain"
        );
        assert_eq!(
            table.resolve(crate::edge::certs::FALLBACK_NAME),
            Some(Upstream::ControlPlane),
            "and locally, which is how somebody fixes a broken node"
        );
    }

    /// A hostname removed from a service must stop routing, or the
    /// name keeps answering after the operator took it away.
    #[tokio::test]
    async fn a_removed_hostname_stops_routing() {
        let (database, _, service) = node().await;
        let port = ports::create(&database, &service, 80, false, Some("api.example.com"))
            .await
            .expect("port");
        services::set_address(&database, &service, Some("10.42.1.5"))
            .await
            .expect("address");

        let table = Arc::new(RouteTable::new());
        sync(&database, Some("node.example"), Some(&table))
            .await
            .expect("sync");

        ports::delete(&database, &port.id).await.expect("delete");
        sync(&database, Some("node.example"), Some(&table))
            .await
            .expect("sync");

        assert_eq!(table.resolve("api.example.com"), None);
    }

    /// A service that publishes TCP but serves no site has nothing to
    /// route: the port is reached by address, not by name.
    #[tokio::test]
    async fn a_published_tcp_port_is_not_a_route() {
        let (database, _, service) = node().await;
        ports::create(&database, &service, 5432, true, None)
            .await
            .expect("port");
        services::set_address(&database, &service, Some("10.42.1.5"))
            .await
            .expect("address");

        let table = Arc::new(RouteTable::new());
        assert_eq!(
            sync(&database, Some("node.example"), Some(&table))
                .await
                .expect("sync"),
            0
        );
    }
}
