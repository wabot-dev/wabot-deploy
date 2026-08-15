//! `wabot-deploy serve` — the daemon.
//!
//! Two listeners and one shutdown. The edge owns both ports; the
//! control plane is reached through it rather than beside it, so
//! there is no second address where the API answers without TLS.

use std::net::SocketAddr;
use std::sync::Arc;

use wabot::lifecycle::{ShutdownPhase, ShutdownTask};
use wabot::prelude::{register_transients, Container};
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
    // And how an authority tells this node an errand is waiting, rather
    // than it finding out on its next fifteen-second pass. Created here
    // for the same reason: the endpoint that rings it and the loop that
    // answers are two halves and neither owns the other.
    let doorbell = Arc::new(crate::network::collect::Doorbell::default());

    // Whatever the last process was in the middle of. An update ends
    // by replacing that process, so the only one who can say whether
    // it worked is this one — see `update::settle_after_restart`.
    crate::update::settle_after_restart(&database).await;

    // What this node is. Here as well as in `install` because an update
    // replaces the binary and restarts without running the installer,
    // so a node upgraded into a version that has this table would
    // otherwise have no row in it — and the console lists that table.
    if let Err(error) = crate::network::ensure_self(&database, &config).await {
        tracing::warn!(%error, "could not record what this node is");
    }
    // And the overlay it is on, if it is on one. Never fatal: a node
    // whose kernel refuses the interface still serves its console,
    // which is where somebody goes to find out why.
    match crate::network::tunnel::ensure(&database, &config).await {
        Ok(Some(overlay)) => tracing::info!(
            address = %overlay.address,
            port = overlay.port,
            peers = overlay.peers,
            "overlay up"
        ),
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "could not bring the overlay up"),
    }

    let container = Container::new();
    // The job handler resolves this. Nothing else registered it —
    // `api::register` takes the database and keeps it inside
    // `NodeStatus` — so the first queued deployment panicked the worker
    // with "no provider registered for type SqliteDatabase". The button
    // answered, the job was stored, and nothing ran: found on the node,
    // because no test covers the join.
    container.register_instance::<wabot::sqlite::SqliteDatabase>(database.clone());

    // Deploying is a job. Registered before anything can enqueue one,
    // and `ensure_ready` because these tables are the store's own —
    // `db.rs` owns the node's schema, the queue brings its two.
    let jobs = Arc::new(wabot_addon_async_sqlite::SqliteJobRepository::new(
        (*database).clone(),
    ));
    let crons = Arc::new(wabot_addon_async_sqlite::SqliteCronJobRepository::new(
        (*database).clone(),
    ));
    jobs.ensure_ready().await?;
    crons.ensure_ready().await?;
    wabot::async_jobs::register_job_repository(&container, jobs);
    wabot::async_jobs::register_cron_job_repository(&container, crons);
    // `InProcessLocker`, which `register_async_runtime` picks when
    // nothing else is registered: there is one node, and the Postgres
    // advisory-lock variant wants a server this product exists not to
    // need.
    wabot::async_jobs::register_async_runtime(&container);
    register_transients!(&container, crate::deploy::jobs::DeployHandler);

    crate::api::register(&container, database.clone());
    // Where a joining node announces itself. On the control plane
    // rather than beside it, so it answers on the node's own hostname
    // and certificate — the endpoint in a join token is the same
    // address the console is at.
    crate::network::api::register(&container, config.clone());
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
        host: crate::node::settings::domain(&database, &config)
            .await
            .unwrap_or_else(|| "localhost".into()),
    });
    crate::registry::register(&container, registry_state);

    // One router: the console's pages, the API's endpoints and the
    // registry answer on the same hostname, because they are the same
    // thing to whoever is looking at the node.
    let control_plane = crate::api::routes(&container)
        .merge(crate::network::api::routes(&container))
        .merge(crate::console::routes(&container))
        .merge(crate::registry::routes(&container));

    let (edge, resolver) =
        crate::edge::build(&database, control_plane, &config, routes.clone()).await?;

    // After the edge is built, so the deployer holds the same route
    // table the listener reads and a deployment takes effect without a
    // restart.
    let deployer = Arc::new(
        crate::deploy::Deployer::new(database.clone(), &config)
            .with_routes(routes)
            .with_certificates(wake.clone()),
    );
    // The job handler resolves this one rather than building a third.
    container.register_instance::<crate::deploy::Deployer>(deployer.clone());
    // The doorbell endpoint rings this; the collector loop answers it.
    container.register_instance::<crate::network::collect::Doorbell>(doorbell.clone());

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
    let errands_database = database.clone();
    let errands_config = config.clone();
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
        .service_with_cancel("reconcile", {
            let reconciler = deployer.clone();
            move |cancel| async move {
                tokio::select! {
                    _ = cancel.cancelled() => return Ok::<(), anyhow::Error>(()),
                    outcome = reconciler.reconcile() => match outcome {
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
            }
        })
        // The bound on what containers write.
        //
        // Its own loop and not a step of anything else: what it protects
        // is the node's disk, which has nothing to do with certificates
        // or with any one service. Cheap enough to run often — a
        // `read_dir` and a `stat` each — and it acts on almost none of
        // the ticks.
        .service_with_cancel("container-logs", {
            let deployer = deployer.clone();
            move |cancel| async move {
                loop {
                    let data_dir = deployer.config().node.data_dir.clone();
                    // On the blocking pool: it reads and rewrites files,
                    // and a log that has grown to eight megabytes is
                    // eight megabytes of read on whatever thread asks.
                    let trimmed = tokio::task::spawn_blocking(move || {
                        crate::deploy::logs::trim_all(&data_dir)
                    })
                    .await
                    .unwrap_or(0);
                    if trimmed > 0 {
                        tracing::info!(trimmed, "container logs trimmed to their last part");
                    }

                    tokio::select! {
                        _ = cancel.cancelled() => return Ok::<(), anyhow::Error>(()),
                        _ = tokio::time::sleep(std::time::Duration::from_secs(300)) => {}
                    }
                }
            }
        })
        // A renewed certificate reaching a running database.
        //
        // Its own loop rather than a step of the certificate loop: that
        // one belongs to `edge`, and having it reach into the deploy
        // path would tie the two together for a check that costs a file
        // comparison. Convergent either way — it asks whether the file
        // matches the store, not whether anything renewed.
        .service_with_cancel("database-certificates", {
            let deployer = deployer.clone();
            move |cancel| async move {
                loop {
                    // Once at start, before the first wait. A node that
                    // was down while a certificate was renewed should
                    // hand it over when it comes back, not a quarter of
                    // an hour later.
                    match deployer.refresh_certificates().await {
                        Ok(0) => {}
                        Ok(refreshed) => {
                            tracing::info!(refreshed, "certificates handed to running databases")
                        }
                        Err(error) => tracing::warn!(%error, "could not refresh certificates"),
                    }

                    tokio::select! {
                        _ = cancel.cancelled() => return Ok::<(), anyhow::Error>(()),
                        _ = tokio::time::sleep(std::time::Duration::from_secs(900)) => {}
                    }
                }
            }
        })
        // Deploys land here. A service that returns ends the process,
        // so the runner's own loop is what keeps this one alive.
        .service(
            "jobs",
            wabot::async_jobs::run_async_workers(
                container.clone(),
                vec![crate::deploy::jobs::DeployHandler::__handler_entry(
                    &container,
                )],
                vec![],
            ),
        )
        .service_with_cancel("edge-https", move |cancel| {
            crate::edge::serve_https(edge, resolver, https, cancel)
        })
        .service_with_cancel("edge-http", move |cancel| {
            crate::edge::serve_http(https_port, http_database, http, cancel)
        })
        // Errands, asked for on a timer. Beside the listeners for the
        // same reason as everything else here: a node whose authority
        // is unreachable must still serve its own console, which is
        // where somebody goes to find out why.
        .service_with_cancel("errands", {
            let database = errands_database.clone();
            let config = errands_config.clone();
            let container = container.clone();
            let doorbell = doorbell.clone();
            move |cancel| {
                crate::network::collect::loop_forever(database, config, container, doorbell, cancel)
            }
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
