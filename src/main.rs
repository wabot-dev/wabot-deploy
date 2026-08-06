//! wabot-deploy — container deployments on a node you own.
//!
//! One binary, three verbs: `install` sets the machine up, `serve` is
//! what systemd runs, `doctor` says what state things are in.
//!
//! Everything is wired by hand on purpose: nothing is discovered, so a
//! component left unregistered is a compile error rather than a route
//! that silently never mounts.

use clap::Parser;

mod api;
mod cli;
mod commands;
mod config;
mod db;
mod ledger;

use cli::{Cli, Command};

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    let config = match config::Config::load(&cli.config) {
        Ok(config) => config,
        Err(error) => {
            // Before tracing is up, because the filter comes *from*
            // the config we just failed to read.
            eprintln!("wabot-deploy: {error}");
            return std::process::ExitCode::from(2);
        }
    };

    init_tracing(&config.log.filter);

    // A control plane is almost entirely waiting — on the network, on
    // containerd, on the disk. tokio's default is one worker per core,
    // which on a 16-core box is 16 stacks for work that never
    // saturates one.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads())
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "could not start the async runtime");
            return std::process::ExitCode::from(2);
        }
    };

    let result = runtime.block_on(async {
        match cli.command {
            Command::Install(args) => commands::install::run(config, &cli.config, args).await,
            Command::Serve => commands::serve::run(config).await,
            Command::Doctor => commands::doctor::run(config, &cli.config).await,
        }
    });

    match result {
        Ok(code) => std::process::ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            tracing::error!("{error:#}");
            std::process::ExitCode::from(1)
        }
    }
}

fn init_tracing(filter: &str) {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        // systemd's journal timestamps every line already, and a
        // duplicate is just noise in `journalctl`.
        .without_time()
        .init();
}

/// `WABOT_DEPLOY_WORKERS`, or two.
fn worker_threads() -> usize {
    std::env::var("WABOT_DEPLOY_WORKERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|count| *count > 0)
        .unwrap_or(2)
}
