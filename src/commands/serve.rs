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

    let container = Container::new();
    crate::api::register(&container, database.clone());
    let control_plane = crate::api::routes(&container);

    let (edge, resolver) = crate::edge::build(&database, control_plane, &config).await?;

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
            crate::edge::acme::renewal_loop(acme_database, acme_config, acme_resolver, cancel)
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
