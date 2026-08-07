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
    // table the listener reads and a deployment takes effect without a
    // restart.
    let deployer = crate::deploy::Deployer::new(database.clone(), &config)
        .with_routes(routes)
        .with_certificates(wake.clone());

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
        // Reconciling is a *service*, not a startup step.
        //
        // It used to run before the listeners, so that a node coming
        // back had its containers before it took traffic for them.
        // That reasoning was wrong in the way that matters: the node
        // reports ready only once everything before the listeners is
        // done, so one slow deployment — an image to unpack, a
        // registry that will not answer — held up readiness until
        // systemd's start timeout killed the process. Then it did it
        // again, in a loop, and the console was down too. The console
        // is where somebody goes to find out why a deployment is
        // stuck; it must not be the thing the stuck deployment takes
        // with it.
        //
        // So the listeners come up first and this runs beside them. A
        // service whose container is not back yet answers 404 for a
        // few seconds instead of not answering at all.
        .service_with_cancel("reconcile", move |cancel| async move {
            tokio::select! {
                _ = cancel.cancelled() => return Ok::<(), anyhow::Error>(()),
                outcome = deployer.reconcile() => match outcome {
                    Ok(0) => {}
                    Ok(started) => tracing::info!(started, "services restored"),
                    Err(error) => tracing::warn!(%error, "could not reconcile services"),
                }
            }

            // Then wait, rather than return. A service that ends takes
            // the process with it — the runner reads it as the system
            // being done — so reconciling and returning shut the node
            // down the moment it finished, seconds after it started.
            cancel.cancelled().await;
            Ok::<(), anyhow::Error>(())
        })
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
