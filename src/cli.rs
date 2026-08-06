//! The three things you can ask the binary to do.
//!
//! One executable rather than three, because they must agree about
//! where the database is and what the config means — and the surest
//! way to keep them agreeing is to give them one copy of the code that
//! decides.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::DEFAULT_CONFIG_PATH;

#[derive(Debug, Parser)]
#[command(
    name = "wabot-deploy",
    version,
    about = "Container deployments on a node you own",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Configuration file. Missing is fine — the defaults describe a
    /// working node.
    #[arg(long, global = true, default_value = DEFAULT_CONFIG_PATH, value_name = "PATH")]
    pub config: PathBuf,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Set the node up. Idempotent: re-running converges rather than
    /// repeating, and a run that failed halfway resumes.
    Install(InstallArgs),

    /// Run the node. This is what systemd starts.
    Serve,

    /// Report what is configured, what is installed, and what is
    /// missing. Read-only — safe on a live node.
    Doctor,
}

#[derive(Debug, clap::Args)]
pub struct InstallArgs {
    /// The hostname this node answers to. Stored in the config, and
    /// what a certificate will later be issued for.
    #[arg(long, value_name = "HOST")]
    pub domain: Option<String>,

    /// Contact address for the certificate authority.
    #[arg(long, value_name = "EMAIL")]
    pub email: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// clap's own consistency checks — a duplicate flag or a bad
    /// default is a panic at startup otherwise, which for an installer
    /// means on the operator's machine rather than ours.
    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn the_config_path_defaults_and_is_global() {
        let cli = Cli::parse_from(["wabot-deploy", "serve"]);
        assert_eq!(cli.config, PathBuf::from(DEFAULT_CONFIG_PATH));

        // `global = true` is what lets the flag follow the subcommand,
        // which is where anyone would naturally type it.
        let cli = Cli::parse_from(["wabot-deploy", "serve", "--config", "/tmp/c.toml"]);
        assert_eq!(cli.config, PathBuf::from("/tmp/c.toml"));
        assert!(matches!(cli.command, Command::Serve));
    }

    #[test]
    fn install_takes_a_domain() {
        let cli = Cli::parse_from(["wabot-deploy", "install", "--domain", "n.example.com"]);
        match cli.command {
            Command::Install(args) => {
                assert_eq!(args.domain.as_deref(), Some("n.example.com"));
                assert!(args.email.is_none());
            }
            other => panic!("expected install, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_subcommand_is_an_error_not_a_default() {
        assert!(Cli::try_parse_from(["wabot-deploy"]).is_err());
    }
}
