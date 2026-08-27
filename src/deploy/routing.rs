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
use crate::errors::PlatformResult;
use crate::repositories::ports;
use crate::repositories::services;

/// Where the edge sends a request for each copy of one service, and
/// which copy each address belongs to.
///
/// **Two different questions live one word apart here**, and both are
/// answered in this list. A copy *here* is reached at the container's
/// own address on the port it listens on. A copy *elsewhere* is reached
/// at that node's overlay address and the port bound there — never the
/// container's own, because a CNI bridge subnet is identical on every
/// node, so a container address names a different container on each
/// machine that reads it.
///
/// One entry per replica, which is the whole of the load balancing: a
/// node running two copies appears twice and takes twice the traffic,
/// and nothing carries a weight.
///
/// Keyed by replica so the console can say *which copy* is not
/// answering. That is why this is one function rather than the two
/// halves it used to be: the page and the router have to derive the
/// same address, and a second derivation is a second answer.
pub fn upstreams_of(
    here: &[crate::entities::replica::Replica],
    elsewhere: &[crate::entities::replica::Replica],
    nodes: &[crate::network::Node],
    service_id: &str,
    container_port: u16,
) -> Vec<(String, SocketAddr)> {
    // No address means no copy is running there. Left out rather than
    // guessed at: a proxy attempt to a dead address is a hung request,
    // where a name with nothing behind it is a truthful 404.
    let mut upstreams: Vec<(String, SocketAddr)> = here
        .iter()
        .filter(|replica| replica.service_id == service_id)
        .filter_map(|replica| {
            let address = replica.address.as_deref()?;
            match format!("{address}:{container_port}").parse::<SocketAddr>() {
                Ok(upstream) => Some((replica.id.clone(), upstream)),
                Err(_) => {
                    tracing::warn!(%address, port = container_port, "unroutable");
                    None
                }
            }
        })
        .collect();

    for replica in elsewhere
        .iter()
        .filter(|replica| replica.service_id == service_id)
        .filter(|replica| !replica.evicted())
    {
        // A copy that has reported no port yet is unreachable, and an
        // invented address here is a request that hangs rather than one
        // that fails over.
        let Some(port) = replica.overlay_port else {
            continue;
        };
        let overlay = replica
            .node_id
            .as_ref()
            .and_then(|node_id| nodes.iter().find(|node| &node.id == node_id))
            .and_then(|node| node.overlay_ip.as_deref());
        let Some(overlay) = overlay else {
            continue;
        };
        match format!("{overlay}:{port}").parse::<SocketAddr>() {
            Ok(upstream) => upstreams.push((replica.id.clone(), upstream)),
            Err(_) => tracing::warn!(%overlay, port, "upstream skipped: not an address"),
        }
    }
    upstreams
}

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
    let placements = crate::repositories::replicas::here(database).await?;
    // And the copies that are *not* here: a name this node answers for
    // reaches every copy of its service, wherever it runs, or placing
    // one elsewhere would quietly halve what the name serves rather
    // than doubling it.
    let nodes = crate::network::all(database).await.unwrap_or_default();
    let elsewhere = crate::repositories::replicas::elsewhere(database).await?;
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
    let serving = crate::repositories::edges::of_node(database, me.as_ref().map(|node| &*node.id))
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
        // **A database's name is not an HTTP name.**
        //
        // An edge terminates TLS and proxies HTTP; Postgres speaks its
        // own protocol on a TCP socket, and a request forwarded to 5432
        // arrives as a startup packet whose length is the ASCII of
        // `GET `. `docs/databases.md` says a database gets no hostname
        // for exactly this reason — and then the naming work gave it
        // one, because a database's name is what `verify-full` checks,
        // and this loop builds a route for *any* port that has one.
        //
        // Found on the node: `orders.db-test.…` in the route table,
        // proxying to `10.42.2.200:5432`. Nothing could work through it
        // and the door was public.
        //
        // Only the route goes. The `service_edge` row stays and is what
        // `acme::wanted_names` reads, so the certificate is still
        // ordered and still served — by the database itself, on its
        // published port, which is where it was always presented from.
        if service.kind.is_managed() {
            continue;
        }
        // Where each copy of this service answers, wherever it runs.
        //
        // One function for both halves — see `upstreams_of`. They used
        // to be built here in two, and the console needed the same
        // answer to say which copy is out of rotation: a third place
        // deriving one address is how the three come to disagree.
        let upstreams: Vec<SocketAddr> = upstreams_of(
            &placements,
            &elsewhere,
            &nodes,
            &service.id,
            port.container_port,
        )
        .into_iter()
        .map(|(_, address)| address)
        .collect();

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
    // And the name every node has, which is how another node reaches this
    // one's control plane over the overlay — the doorbell, and whatever
    // phase 9 builds on it. Without a route the edge answers 404 by
    // hostname and the request never reaches the API at all: measured, by
    // ringing a doorbell that could not be reached.
    //
    // The certificate half of this is `certs::own_names`. Both halves are
    // needed and neither is visible from the other, which is what made the
    // 404 read as "the endpoint is not registered".
    let internal = crate::network::me(database)
        .await
        .ok()
        .flatten()
        .map(|me| crate::network::internal_name(&me.id));
    // And the addresses of this machine's own interfaces.
    //
    // **This is what makes a fresh private node reachable at all.** The
    // edge falls back to the control plane only while *no* route exists,
    // and that answer is worked out once at startup — so the three rows
    // above are enough to take the fallback away, and the next restart
    // answers 404 to the bare address the operator was using. It looked
    // like it worked and then stopped. Naming the node after its own
    // address was the workaround, and it costs more than it fixes:
    // `ensure_self` would publish a private address as this node's
    // endpoint and the node would pass for public.
    //
    // Only the console answers there — a service is reached by its
    // hostname or its published port — and `network::lan` leaves out the
    // platform's own `10.42.0.0/16`, which is what stops a container
    // opening the console at its bridge's gateway.
    let own: Vec<String> = node_domain
        .map(str::to_string)
        .into_iter()
        .chain(internal)
        .chain(Some(crate::edge::certs::FALLBACK_NAME.to_string()))
        .chain(crate::network::lan::addresses())
        .collect();
    for name in &own {
        routes::upsert(database, name, &Upstream::ControlPlane, None).await?;
    }
    // A laptop that changed network keeps its old address as a row for
    // ever otherwise. Harmless in itself — the row points at this node's
    // own console — but it is a name this node claims and no longer
    // answers on, and `doctor` would list it for ever.
    routes::retain_control_plane(database, &own).await?;

    // And one row per tenant that has a domain, so its console answers
    // there and is narrowed to it before any handler runs.
    //
    // **Its own convergent pass**, and that is not tidiness. A tenant
    // row falls in the gap between `retain_proxies`, which skips a route
    // with no `service_id` on purpose, and `retain_control_plane`, which
    // touches only rows of its own kind — the same gap that left an
    // errand-written proxy row behind for ever until `forget_for_other`
    // was written. A tenant renamed, given a different domain or deleted
    // would otherwise keep answering on the name it used to have, with a
    // certificate for it, for as long as the node ran.
    let tenants = crate::repositories::tenants::all(database).await?;
    let mut tenanted: Vec<String> = Vec::new();
    for tenant in tenants {
        let Some(domain) = tenant.domain else {
            continue;
        };
        // Never over one of this node's own names. The console an
        // operator fixes things from is the one thing that must not be
        // reachable only through somebody else's frontier, and a tenant
        // given the node's domain would take it.
        if own.iter().any(|name| name.eq_ignore_ascii_case(&domain)) {
            tracing::warn!(%domain, tenant = %tenant.id, "a tenant claims one of this node's names");
            continue;
        }
        routes::upsert(database, &domain, &Upstream::Tenant(tenant.id), None).await?;
        tenanted.push(domain);
    }
    routes::retain_tenants(database, &tenanted).await?;

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
        let replicas = crate::repositories::replicas::of_service(database, service_id)
            .await
            .expect("replicas");
        crate::repositories::replicas::set_address(database, &replicas[0].id, address)
            .await
            .expect("address");
    }

    use super::*;

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
        let project = crate::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        let service = crate::commands::create_service::CreateService {
            project_id: &project.id,
            name: "web",
            image: "docker.io/library/nginx:alpine",
            env: &[],
        }
        .run(&database)
        .await
        .expect("service");
        (database, project.id, service.id)
    }

    /// The console answers on this machine's own addresses.
    ///
    /// **The bug this fixes looked like it worked and then stopped.** The
    /// edge falls back to the control plane only while *no* route exists,
    /// and that answer is worked out once at startup — so the three
    /// control-plane rows this function writes were themselves enough to
    /// take the fallback away, and the next restart answered 404 to the
    /// bare address the operator had been using from another machine on
    /// the same network. Found by Jorge, who worked around it by naming
    /// the node after its own address — which also makes `ensure_self`
    /// publish a private address as this node's endpoint.
    #[tokio::test]
    async fn the_console_answers_on_this_machines_own_addresses() {
        let (database, _, _) = node().await;
        let table = Arc::new(RouteTable::new());
        sync(&database, Some("node.example"), Some(&table))
            .await
            .expect("sync");

        for address in crate::network::lan::addresses() {
            assert_eq!(
                table.resolve(&address),
                Some(crate::edge::routes::Upstream::ControlPlane),
                "no route for this machine's own {address}"
            );
        }
        // And the two that were always there.
        assert_eq!(
            table.resolve("localhost"),
            Some(crate::edge::routes::Upstream::ControlPlane)
        );
        assert_eq!(
            table.resolve("node.example"),
            Some(crate::edge::routes::Upstream::ControlPlane)
        );

        // Nothing from the platform's own space: `10.42.0.0/16` holds
        // `wabot0` and the gateway of every project bridge, so a row for
        // one of those would be a container opening the node's console.
        assert!(
            table.hosts().iter().all(|host| !host.starts_with("10.42.")),
            "a platform address became a name: {:?}",
            table.hosts()
        );
    }

    /// An address this machine no longer holds stops being a name it
    /// claims.
    ///
    /// A laptop that changes network would otherwise keep the old one for
    /// ever — harmless in itself, since the row points at this node's own
    /// console, but it is a name it answers for and cannot be reached at,
    /// and `doctor` would list it as long as the node lived.
    #[tokio::test]
    async fn an_address_this_machine_gave_up_is_not_a_name_any_more() {
        let (database, _, _) = node().await;
        crate::edge::routes::upsert(
            &database,
            "192.0.2.77",
            &crate::edge::routes::Upstream::ControlPlane,
            None,
        )
        .await
        .expect("a name from a network this machine has left");

        let table = Arc::new(RouteTable::new());
        sync(&database, Some("node.example"), Some(&table))
            .await
            .expect("sync");

        assert_eq!(table.resolve("192.0.2.77"), None, "{:?}", table.hosts());
        // And the one that is never dropped, whatever else goes.
        assert_eq!(
            table.resolve("localhost"),
            Some(crate::edge::routes::Upstream::ControlPlane),
            "localhost is how somebody reaches the console when everything else is wrong"
        );
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
        ports::create(&database, &service, 80, false, Some("api.example.com"), &[])
            .await
            .expect("port");
        running_at(&database, &service, Some("10.42.1.5")).await;

        // Somebody else answers for it now.
        crate::repositories::edges::set(
            &database,
            &service,
            "api.example.com",
            &["nd-other".into()],
        )
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

    /// The name another node dials this one by has to reach the control
    /// plane, or the edge answers 404 by hostname and the request never
    /// gets to the API.
    ///
    /// Found by ringing a doorbell that could not be reached: the endpoint
    /// was registered, the certificate covered the name, the tunnel was up,
    /// and the answer was 404 — because routing is a separate half and
    /// **A database's name is not an HTTP name**, and this loop built a
    /// route for any port that had one.
    ///
    /// Found on a node: `orders.db-test.…` in the route table pointing
    /// at `10.42.2.200:5432`. An edge terminates TLS and proxies HTTP;
    /// a request forwarded there arrives at Postgres as a startup packet
    /// whose length is the ASCII of `GET `. Nothing could ever work
    /// through that door and it was public.
    ///
    /// `docs/databases.md` says a database gets no hostname for exactly
    /// this reason — and then the naming work gave it one, because a
    /// name is what `verify-full` checks. Both are right; what was
    /// missing was this line.
    ///
    /// The certificate is the other half of the claim and it must not
    /// move: `acme::wanted_names` reads `service_edge`, not the route
    /// table, so the name is still ordered for and still served — by the
    /// database itself, on its published port.
    #[tokio::test]
    async fn a_database_gets_a_certificate_and_never_an_http_route() {
        let database = crate::db::open_in_memory().await.expect("open");
        let table = Arc::new(RouteTable::new());
        // A node that can answer for a name at all, which is what
        // `ports::serve_here` asks before writing the row.
        crate::network::ensure_self(&database, &crate::config::Config::default())
            .await
            .expect("self");
        let mut me = crate::network::me(&database)
            .await
            .expect("query")
            .expect("there");
        me.kind = crate::network::Kind::Public;
        me.endpoint = Some("example.com:443".into());
        crate::network::save(&database, &me).await.expect("saved");

        let project = crate::repositories::projects::create(&database, "db-test")
            .await
            .expect("project");
        let (service, _) = crate::commands::create_database::CreateDatabase {
            project_id: &project.id,
            name: "orders",
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&database)
        .await
        .expect("database");

        // The state a database is in once it has a name and is running:
        // a hostname on its port row, an address on its copy.
        let name = "orders.db-test.example.com";
        let port = crate::repositories::ports::of_service(&database, &service.id)
            .await
            .expect("query")
            .pop()
            .expect("a database has one port");
        crate::repositories::ports::set_hostname(&database, &port.id, Some(name))
            .await
            .expect("named");
        let replica = crate::repositories::replicas::in_slot(&database, &service.id, 1)
            .await
            .expect("query")
            .expect("its primary");
        crate::repositories::replicas::set_address(&database, &replica.id, Some("10.42.2.200"))
            .await
            .expect("address");

        sync(&database, Some("example.com"), Some(&table))
            .await
            .expect("synced");

        assert!(
            table.resolve(name).is_none(),
            "a database name must not be an HTTP route: {:?}",
            table.hosts()
        );
        // And the row that decides the certificate is untouched, which
        // is what keeps `verify-full` working against the name.
        assert!(
            crate::repositories::edges::of_node(
                &database,
                crate::network::me(&database)
                    .await
                    .expect("query")
                    .as_ref()
                    .map(|node| &*node.id),
            )
            .await
            .expect("query")
            .contains(&name.to_string()),
            "this node still answers for the name, which is what orders its certificate"
        );
    }

    /// neither half is visible from the other.
    #[tokio::test]
    async fn the_name_another_node_dials_reaches_the_control_plane() {
        let database = crate::db::open_in_memory().await.expect("open");
        let me = crate::network::ensure_self(&database, &crate::config::Config::default())
            .await
            .expect("seeded");

        sync(&database, None, None).await.expect("synced");

        let name = crate::network::internal_name(&me.id);
        let routed = crate::edge::routes::load_all(&database)
            .await
            .expect("routes")
            .into_iter()
            .find(|(host, _)| host == &name)
            .map(|(_, upstream)| upstream)
            .expect("the name another node dials has no route");
        assert!(
            matches!(routed, crate::edge::routes::Upstream::ControlPlane),
            "it has to reach this node's own API: {routed:?}"
        );
    }

    #[tokio::test]
    async fn a_running_service_with_a_hostname_gets_a_route() {
        let (database, _, service) = node().await;
        ports::create(&database, &service, 80, false, Some("api.example.com"), &[])
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
        ports::create(&database, &service, 80, false, Some("api.example.com"), &[])
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
        ports::create(&database, &service, 80, false, Some("api.example.com"), &[])
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
        let port = ports::create(&database, &service, 80, false, Some("api.example.com"), &[])
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
        ports::create(&database, &service, 5432, true, None, &[])
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
