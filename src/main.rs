//! wabot-deploy
//!
//! Run it with `cargo run`. Everything is wired by hand on purpose:
//! nothing is discovered, so a component you forget to register is a
//! compile error rather than a route that silently never mounts.

use wabot::prelude::*;
use wabot::rest::{run_rest_controllers, RestServerConfig};
mod api;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `.env` is read before anything else looks at the environment.
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wabot=debug".into()),
        )
        .init();

    let container = Container::new();

    // The pool the framework's stores share. `register_locker` swaps the
    // in-process lock for Postgres advisory locks, which is what makes
    // running more than one instance safe.
    let database = wabot::pg::PgDatabase::connect(wabot::pg::PgConfig::from_env()?).await?;
    database.ping().await?;
    wabot::async_jobs::register_locker(
        &container,
        wabot::pg::PgLocker::arc(database.pool().clone()),
    );

    api::register(&container);

    let router = api::routes(&container);

    run_rest_controllers(router, RestServerConfig::from_env()).await?;
    Ok(())
}
