//! `wabot-deploy serve` — the daemon.
//!
//! For now this is the control plane on a plain HTTP port. The edge
//! (TLS on 443, host routing, the proxy) lands next and replaces the
//! listener; the composition here is the shape it plugs into.

use std::sync::Arc;

use wabot::lifecycle::{ShutdownPhase, ShutdownTask};
use wabot::prelude::Container;
use wabot::rest::{run_rest_controllers, RestServerConfig};
use wabot::ProjectRunner;

use crate::config::Config;

pub async fn run(config: Config) -> anyhow::Result<i32> {
    let database = Arc::new(crate::db::open(&config.database_path()).await?);
    database.ping().await?;

    let container = Container::new();
    crate::api::register(&container, database.clone());
    let router = crate::api::routes(&container);

    // Deliberately not 443 yet: the control plane binds a plain port
    // until the edge exists, and pretending otherwise by listening on
    // 443 without TLS would be worse than being obviously incomplete.
    let bind = format!("127.0.0.1:{}", control_plane_port(&config)).parse()?;
    tracing::info!(%bind, version = crate::api::VERSION, "starting");

    let runner = ProjectRunner::new(container.clone());
    let closing = database.clone();

    let outcome = runner
        .service_with_cancel("rest", move |cancel| {
            run_rest_controllers(router, RestServerConfig::new(bind).with_shutdown(cancel))
        })
        // Close phase, after the drain: checkpointing the WAL while a
        // request might still write it would leave work behind for the
        // next start, which is the thing this avoids.
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

/// Until the edge arrives, `edge.https_port` names the port the
/// control plane answers on — one number for an operator to think
/// about rather than two that will merge later.
fn control_plane_port(config: &Config) -> u16 {
    config.edge.https_port
}
