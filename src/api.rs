//! The control-plane HTTP surface.
//!
//! Two endpoints for now, and the split between them is the point.
//! `/healthz` answers whether the *process* is alive; `/readyz`
//! answers whether it can do its job. Collapsing them into one is how
//! a node with an unreachable database keeps receiving traffic,
//! because the thing being asked is "are you running" and it is.

use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;
use wabot::prelude::*;
use wabot::rest::axum::Router;
use wabot::rest::{RestError, RestResult};
use wabot::sqlite::SqliteDatabase;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Process-wide facts the health endpoints report.
///
/// Registered with `register_instance` rather than `#[singleton]`:
/// the start time is captured when the process boots, not the first
/// time something resolves it, and a container-built default would
/// report an uptime that began at the first health check.
pub struct NodeStatus {
    started: Instant,
    database: Arc<SqliteDatabase>,
}

impl NodeStatus {
    pub fn new(database: Arc<SqliteDatabase>) -> Self {
        Self {
            started: Instant::now(),
            database,
        }
    }

    fn uptime_seconds(&self) -> u64 {
        self.started.elapsed().as_secs()
    }
}

/// `&'static str` rather than `String`: these are constants, and the
/// response type exists to be *written*. It deliberately does not
/// derive `Deserialize` — a borrowed field can only satisfy
/// `Deserialize<'static>`, so a client wanting to parse this should
/// declare its own owned struct.
#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct Ready {
    pub status: &'static str,
    pub database: &'static str,
}

#[singleton]
pub struct HealthController {
    node: Arc<NodeStatus>,
}

#[rest_controller("/")]
impl HealthController {
    /// Liveness. Touches nothing, so it stays true for exactly as long
    /// as the process can answer at all — which is the only question
    /// a restart policy should be asking.
    #[get("/healthz")]
    async fn healthz(&self) -> RestResult<Health> {
        Ok(Health {
            status: "ok",
            version: VERSION,
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

/// Nothing is discovered: a type left out here panics on resolve.
pub fn register(container: &Container, database: Arc<SqliteDatabase>) {
    container.register_instance::<NodeStatus>(Arc::new(NodeStatus::new(database)));
    register_singletons!(container, HealthController);
}

pub fn routes(container: &Container) -> Router {
    HealthController::register_routes(container, Router::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wabot::rest::axum::http::StatusCode;
    use wabot::testing::RestHarness;

    async fn harness() -> RestHarness {
        let database = crate::db::open_in_memory().await.expect("open");
        let container = Container::new();
        register(&container, Arc::new(database));
        RestHarness::new(routes(&container))
    }

    #[tokio::test]
    async fn healthz_reports_the_version() {
        let response = harness().await.get("/healthz").send().await;
        response.assert_ok();
        let health = response.value();
        assert_eq!(health["status"], "ok");
        assert_eq!(health["version"], VERSION);
        assert!(health["uptime_seconds"].is_u64());
    }

    #[tokio::test]
    async fn readyz_passes_when_the_database_answers() {
        let response = harness().await.get("/readyz").send().await;
        response.assert_ok();
        assert_eq!(response.value()["database"], "ok");
    }

    /// Liveness and readiness must be separately reachable, or the
    /// distinction they exist for is not actually wired up.
    #[tokio::test]
    async fn the_two_checks_are_different_endpoints() {
        let harness = harness().await;
        assert!(harness.get("/healthz").send().await.is_success());
        assert!(harness.get("/readyz").send().await.is_success());
        harness
            .get("/nope")
            .send()
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }

    // Not tested here: readiness returning 503 when the database is
    // unreachable. SQLite keeps answering from an open handle after the
    // file is deleted, so the only way to stage it is a fault-injecting
    // database — more machinery than the three lines of mapping it
    // would cover. It becomes worth testing when readiness grows a
    // second dependency that *can* fail on demand.
}
