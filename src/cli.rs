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

    /// Contact address for the certificate authority. It mails this
    /// before a certificate expires, which is the last warning before
    /// an outage.
    #[arg(long, value_name = "EMAIL")]
    pub email: Option<String>,

    /// Use Let's Encrypt's staging environment.
    ///
    /// Its certificates are untrusted by design, but production
    /// refuses more than five failed orders per hour — so debugging a
    /// DNS problem against production locks you out for the rest of
    /// the hour.
    #[arg(long)]
    pub acme_staging: bool,

    /// Install the node but leave containerd alone.
    ///
    /// For a machine where containerd is managed by something else, or
    /// where you want the control plane up before the runtime.
    #[arg(long)]
    pub no_runtime: bool,

    /// Change nothing outside the data directory and the config file.
    ///
    /// No containerd, no `/usr/local/bin`, no systemd unit, nothing
    /// started. What is left is a configured node you can run in the
    /// foreground — which is what a container image wants, and what a
    /// test can afford to do on somebody's laptop.
    #[arg(long)]
    pub no_system: bool,

    /// Do everything except start the service.
    ///
    /// For an install that should take effect on the next boot rather
    /// than now.
    #[arg(long)]
    pub no_start: bool,

    /// Skip the machine checks.
    ///
    /// They exist because each one becomes a confusing failure later.
    /// Skipping is for when you know something the check does not.
    #[arg(long)]
    pub skip_preflight: bool,
}

impl InstallArgs {
    /// Nothing set — what a bare `install` means.
    ///
    /// For tests, so that adding a flag does not mean editing every
    /// one of them to say "and not that either".
    #[cfg(test)]
    pub fn none() -> Self {
        Self {
            domain: None,
            email: None,
            acme_staging: false,
            no_runtime: false,
            // Tests must not touch containerd, systemd, or
            // /usr/local on whoever's machine is running them.
            no_system: true,
            no_start: false,
            // And they run as an ordinary user, which the real
            // preflight correctly refuses.
            skip_preflight: true,
        }
    }

    #[cfg(test)]
    pub fn with_domain(domain: &str) -> Self {
        Self {
            domain: Some(domain.to_string()),
            ..Self::none()
        }
    }
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
