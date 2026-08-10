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
    // Addresses are per replica now: a service is *n* running copies,
    // and which of them a name reaches is the route table's question.
    let placements = crate::platform::replicas::here(database).await?;
    // And the copies that are *not* here: a name this node answers for
    // reaches every copy of its service, wherever it runs, or placing
    // one elsewhere would quietly halve what the name serves rather
    // than doubling it.
    let nodes = crate::network::all(database).await.unwrap_or_default();
    let elsewhere = crate::platform::replicas::elsewhere(database).await?;
    // Which of this node's own names it was asked to answer for.
    //
    // It used to answer for all of them, automatically, because the
    // node that owns a service was assumed to be the one exposing it.
    // That is not the model: the only thing separating a private node
    // from a public one is whether it exposes its own address, so a
    // node can perfectly well own a service that is served from
    // somewhere else — and building a route here for a name pointing at
    // another machine would mean this node answering for a name it was
    // not chosen for.
    let me = crate::network::me(database).await.ok().flatten();
    let serving = crate::platform::edges::of_node(database, me.as_ref().map(|node| &*node.id))
        .await
        .unwrap_or_default();

    let mut hosts = Vec::new();
    for port in &ports {
        let Some(hostname) = &port.hostname else {
            continue;
        };
        if !serving.contains(hostname) {
            continue;
        }
        let Some(service) = services.iter().find(|s| s.id == port.service_id) else {
            continue;
        };
        // No address means no copy of it is running here. The route is
        // dropped rather than kept pointing at where it used to be: a
        // 404 from the edge is a truthful answer, and a proxy attempt
        // to a dead address is a hung request.
        //
        // **One entry per replica**, not per node. A node running two
        // copies contributes two, so a plain round-robin sends it twice
        // the requests — which is what "two there and one here" is
        // asking for, without anything having to carry a weight.
        let mut upstreams: Vec<SocketAddr> = placements
            .iter()
            .filter(|replica| replica.service_id == service.id)
            .filter_map(|replica| replica.address.as_deref())
            .filter_map(|address| {
                match format!("{address}:{}", port.container_port).parse::<SocketAddr>() {
                    Ok(upstream) => Some(upstream),
                    Err(_) => {
                        tracing::warn!(%address, port = port.container_port, "unroutable");
                        None
                    }
                }
            })
            .collect();

        // The copies on other nodes, reached the only way anything
        // outside a node can: that node's overlay address and the port
        // it opened there. Appended to the local ones rather than
        // replacing them, so the list is one entry per replica wherever
        // it runs — which is what makes a node with two of them take
        // twice the traffic without anything carrying a weight.
        let mine: Vec<_> = elsewhere
            .iter()
            .filter(|replica| replica.service_id == service.id)
            .cloned()
            .collect();
        upstreams.extend(
            crate::platform::edges::upstreams(&mine, &nodes)
                .iter()
                .filter_map(|address| match address.parse::<SocketAddr>() {
                    Ok(upstream) => Some(upstream),
                    Err(_) => {
                        tracing::warn!(%address, "upstream skipped: not an address");
                        None
                    }
                }),
        );

        // Nothing running: the route is dropped rather than kept
        // pointing at where it used to be. A 404 from the edge is a
        // truthful answer; a proxy attempt to a dead address is a hung
        // request.
        if upstreams.is_empty() {
            continue;
        }

        routes::upsert(
            database,
            hostname,
            &Upstream::Proxy(upstreams),
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
    /// Put the service's copy on an address, the way a deployment does.
    ///
    /// An address belongs to a replica now — a service is *n* running
    /// copies — so a test that sets one has to say which copy.
    async fn running_at(
        database: &wabot::sqlite::SqliteDatabase,
        service_id: &str,
        address: Option<&str>,
    ) {
        let replicas = crate::platform::replicas::of_service(database, service_id)
            .await
            .expect("replicas");
        crate::platform::replicas::set_address(database, &replicas[0].id, address)
            .await
            .expect("address");
    }

    use super::*;
    use crate::platform::projects;

    /// A node with a name of its own, which is what makes it able to
    /// answer for one. Without the self row nothing here is an edge for
    /// anything — correctly, a node that does not know who it is cannot
    /// have been chosen — and every route would be skipped.
    async fn node() -> (SqliteDatabase, String, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        crate::node::settings::set_domain(&database, Some("node.example"))
            .await
            .expect("domain");
        crate::network::ensure_self(&database, &crate::config::Config::default())
            .await
            .expect("self");
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

    /// A node can own a service and not be the one exposing it — the
    /// only thing separating a private node from a public one is
    /// whether it exposes its own address. So a name this node was not
    /// chosen to answer for gets no route here, however local the
    /// service is: building one would be this node answering for a name
    /// somebody pointed at another machine.
    #[tokio::test]
    async fn a_name_this_node_was_not_chosen_for_is_not_routed_here() {
        let (database, _, service) = node().await;
        ports::create(&database, &service, 80, false, Some("api.example.com"))
            .await
            .expect("port");
        running_at(&database, &service, Some("10.42.1.5")).await;

        // Somebody else answers for it now.
        crate::platform::edges::set(&database, &service, "api.example.com", &["nd-other".into()])
            .await
            .expect("set");

        let table = Arc::new(RouteTable::new());
        sync(&database, Some("node.example"), Some(&table))
            .await
            .expect("sync");

        assert_eq!(
            table.resolve("api.example.com"),
            None,
            "this node answered for a name it was not chosen for"
        );
    }

    #[tokio::test]
    async fn a_running_service_with_a_hostname_gets_a_route() {
        let (database, _, service) = node().await;
        ports::create(&database, &service, 80, false, Some("api.example.com"))
            .await
            .expect("port");
        running_at(&database, &service, Some("10.42.1.5")).await;

        let table = Arc::new(RouteTable::new());
        assert_eq!(
            sync(&database, Some("node.example"), Some(&table))
                .await
                .expect("sync"),
            1
        );

        assert_eq!(
            table.resolve("api.example.com"),
            Some(Upstream::Proxy(vec!["10.42.1.5:80".parse().unwrap()]))
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
        running_at(&database, &service, Some("10.42.1.5")).await;

        let table = Arc::new(RouteTable::new());
        sync(&database, Some("node.example"), Some(&table))
            .await
            .expect("sync");

        // Stopped: the address goes, and so must the route.
        running_at(&database, &service, None).await;
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
        running_at(&database, &service, Some("10.42.1.5")).await;

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
        running_at(&database, &service, Some("10.42.1.5")).await;

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
        running_at(&database, &service, Some("10.42.1.5")).await;

        let table = Arc::new(RouteTable::new());
        assert_eq!(
            sync(&database, Some("node.example"), Some(&table))
                .await
                .expect("sync"),
            0
        );
    }
}
