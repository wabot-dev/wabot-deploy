//! Whether this node is up, and whether it is ready.
//!
//! Two routes and no session: they are what a load balancer and a
//! restart script ask, and neither has one.

use crate::api::*;

use std::sync::Arc;
use wabot::prelude::*;
use wabot::rest::{RestError, RestResult};

#[singleton]
pub struct HealthRestController {
    node: Arc<NodeStatus>,
}

#[rest_controller("/")]
impl HealthRestController {
    /// Liveness. Touches nothing, so it stays true for exactly as long
    /// as the process can answer at all — which is the only question
    /// a restart policy should be asking.
    #[get("/healthz")]
    async fn healthz(&self) -> RestResult<Health> {
        Ok(Health {
            status: "ok",
            version: crate::update::VERSION,
            uptime_seconds: self.node.uptime_seconds(),
        })
    }

    /// Readiness. Round-trips the database, because a node that cannot
    /// read its own state cannot serve a deployment either.
    #[get("/readyz")]
    async fn readyz(&self) -> RestResult<Ready> {
        self.node.database.ping().await.map_err(|error| {
            tracing::warn!(%error, "readiness check failed");
            // 503, not 500: this is "not yet" or "not any more", and a
            // load balancer should take the node out rather than
            // conclude the request was malformed.
            RestError::Client {
                status: 503,
                message: "database unavailable".into(),
            }
        })?;
        Ok(Ready {
            status: "ready",
            database: "ok",
        })
    }
}
