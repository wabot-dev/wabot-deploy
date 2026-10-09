//! wabot-deploy — container deployments on a node you own.
//!
//! One binary: `install` sets the machine up, `serve` is what systemd
//! runs, `doctor` says what state things are in, and `join` makes this
//! node part of somebody else's network.
//!
//! Everything is wired by hand on purpose: nothing is discovered, so a
//! component left unregistered is a compile error rather than a route
//! that silently never mounts.

use clap::Parser;

mod cli;
mod domain;
mod features;
mod infra;
#[cfg(test)]
mod tree;
mod web;

use cli::{Cli, Command};

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    let config = match infra::config::Config::load(&cli.config) {
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
            Command::Install(args) => cli::install::run(config, &cli.config, args).await,
            Command::Serve => cli::serve::run(config).await,
            Command::Doctor => cli::doctor::run(config, &cli.config).await,
            Command::Join { token } => cli::join::run(config, &token).await,
            Command::SetupToken => cli::setup_token::run(config).await,
            Command::Passwd { username } => cli::passwd::run(config, &username).await,
            Command::Backup { out } => cli::backup::run(config, out).await,
            Command::Clean { apply, volumes } => cli::clean::run(config, apply, volumes).await,
            Command::LogWriter { path } => cli::log_writer::run(path),
            Command::RestoreNode {
                from,
                same_node,
                new_node,
                over_my_dead_body,
            } => {
                let identity = match (same_node, new_node) {
                    (true, _) => Some(cli::backup::Identity::Same),
                    (_, true) => Some(cli::backup::Identity::New),
                    _ => None,
                };
                cli::backup::restore_node(config, from, identity, over_my_dead_body).await
            }
            Command::Restore {
                database,
                to,
                into,
                from,
            } => cli::backup::restore(config, database, to, into, from).await,
            Command::Containerd { pull, run, port } => cli::containerd::run(pull, run, port).await,
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
