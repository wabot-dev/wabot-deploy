//! `wabot-deploy serve` — the daemon.
//!
//! Two listeners and one shutdown. The edge owns both ports; the
//! control plane is reached through it rather than beside it, so
//! there is no second address where the API answers without TLS.

use std::net::SocketAddr;
use std::sync::Arc;

use wabot::lifecycle::{ShutdownPhase, ShutdownTask};
use wabot::prelude::Container;
use wabot::ProjectRunner;

use crate::config::Config;

pub async fn run(config: Config) -> anyhow::Result<i32> {
    let database = Arc::new(crate::db::open(&config.database_path()).await?);
    database.ping().await?;

    // One route table, created before either half that uses it: the
    // edge reads it on every request and the deployer replaces it
    // whole after a deployment. Two tables would mean a deployment
    // that changes nothing until the next restart.
    let routes = Arc::new(crate::edge::routes::RouteTable::new());
    // How a deployment tells the certificate loop that a hostname
    // appeared. Created here because both halves need it and neither
    // owns the other.
    let wake = Arc::new(crate::edge::acme::Wake::default());

    let container = Container::new();
    crate::api::register(&container, database.clone());
    crate::console::register(
        &container,
        database.clone(),
        config.clone(),
        routes.clone(),
        wake.clone(),
    );

    // The registry shares the control plane's hostname and its
    // listener: `docker push node.example/project/app` reaches the
    // same place the console does, over the same certificate, with no
    // second port to open.
    let registry_state = Arc::new(crate::registry::RegistryState {
        database: database.clone(),
        deployer: Arc::new(
            crate::deploy::Deployer::new(database.clone(), &config)
                .with_routes(routes.clone())
                .with_certificates(wake.clone()),
        ),
        host: config
            .node
            .domain
            .clone()
            .unwrap_or_else(|| "localhost".into()),
    });
    crate::registry::register(&container, registry_state);

    // One router: the console's pages, the API's endpoints and the
    // registry answer on the same hostname, because they are the same
    // thing to whoever is looking at the node.
    let control_plane = crate::api::routes(&container)
        .merge(crate::console::routes(&container))
        .merge(crate::registry::routes(&container));

    let (edge, resolver) =
        crate::edge::build(&database, control_plane, &config, routes.clone()).await?;

    // After the edge is built, so the deployer holds the same route
    // table the listener reads — a deployment then takes effect
    // without a restart. Before the listener *starts*, so a node
    // coming back up has its containers before it takes traffic for
    // them.
    //
    // Never fatal: a node that cannot reach containerd must still
    // serve the console, which is where somebody would go to find out
    // why it cannot.
    let deployer = crate::deploy::Deployer::new(database.clone(), &config)
        .with_routes(routes)
        .with_certificates(wake.clone());
    match deployer.reconcile().await {
        Ok(0) => {}
        Ok(started) => tracing::info!(started, "services restored"),
        Err(error) => tracing::warn!(%error, "could not reconcile services"),
    }

    let https: SocketAddr = (bind_address(&config), config.edge.https_port).into();
    let http: SocketAddr = (bind_address(&config), config.edge.http_port).into();
    tracing::info!(
        %https,
        %http,
        version = crate::api::VERSION,
        certificates = resolver.names().join(", "),
        "starting"
    );

    let closing = database.clone();
    let acme_database = database.clone();
    let acme_config = config.clone();
    let acme_resolver = resolver.clone();
    let acme_wake = wake.clone();
    let http_database = database.clone();
    let https_port = config.edge.https_port;

    let outcome = ProjectRunner::new(container.clone())
        .service_with_cancel("edge-https", move |cancel| {
            crate::edge::serve_https(edge, resolver, https, cancel)
        })
        .service_with_cancel("edge-http", move |cancel| {
            crate::edge::serve_http(https_port, http_database, http, cancel)
        })
        // Certificates are obtained *beside* the listeners, never
        // before them: the HTTP-01 challenge is a request this node has
        // to answer, so ordering issuance ahead of the listener would
        // deadlock on itself.
        .service_with_cancel("acme", move |cancel| {
            crate::edge::acme::renewal_loop(
                acme_database,
                acme_config,
                acme_resolver,
                acme_wake,
                cancel,
            )
        })
        // Close phase, after the drain: checkpointing the WAL while a
        // request might still write to it leaves work for the next
        // start, which is the thing this avoids.
        .on_shutdown(ShutdownTask::new(
            "database",
            ShutdownPhase::Close,
            move || {
                let database = closing.clone();
                async move {
                    if let Err(error) = database.close().await {
                        tracing::warn!(%error, "closing the database");
                    }
                }
            },
        ))
        .run()
        .await;

    Ok(outcome.exit_code())
}

/// Where to listen.
///
/// `0.0.0.0` on a real node — the edge terminates TLS, so this is the
/// address the world is meant to reach. Loopback only when the ports
/// are unprivileged, which in practice means a developer running it by
/// hand: an unauthenticated console should not appear on the network
/// of whatever laptop it was started on.
fn bind_address(config: &Config) -> std::net::IpAddr {
    if config.edge.https_port < 1024 {
        std::net::Ipv4Addr::UNSPECIFIED.into()
    } else {
        std::net::Ipv4Addr::LOCALHOST.into()
    }
}
