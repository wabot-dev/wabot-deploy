//! What you can ask the binary to do.
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

    /// Take instructions from another node, using a token it minted.
    ///
    /// The sibling of `install`: that one makes this machine a node,
    /// this one makes it part of somebody's network. Both ends treat a
    /// second attempt with the same token as the same join, so a run
    /// that failed halfway is fixed by running it again.
    Join {
        /// The token the other node's console showed. It begins with
        /// `wdj1.` and it works once.
        #[arg(value_name = "TOKEN")]
        token: String,
    },

    /// Issue a setup token and print it.
    ///
    /// `install` prints one already. This is for the token that
    /// expired, or the terminal buffer that is gone — and for the
    /// operator who has to hand one to somebody else.
    SetupToken,

    /// Give an account a new password, and print it.
    ///
    /// Because a node whose only administrator forgot their password had
    /// no way back in: the setup token refuses once an account exists —
    /// deliberately, it is what stops a second administrator being minted
    /// — and nothing else could write a password. The console was locked
    /// and the machine was fine.
    ///
    /// This grants nothing new. Whoever runs it already has root on the
    /// box, which means the binary, the database and every container; a
    /// recovery path that root does not have is a lock on a door with no
    /// wall. What it does is make the recovery obvious instead of a
    /// reinstall.
    Passwd {
        /// Whose. Matched the way signing in matches it — case-insensitive.
        #[arg(value_name = "USERNAME")]
        username: String,
    },

    /// Copy everything this node would need to be itself again.
    ///
    /// The database, and every volume a managed engine does not own —
    /// see `commands::backup` for why those are named and skipped
    /// rather than copied wrongly. Images are deliberately left out:
    /// they are in a registry and come back with a pull.
    ///
    /// A directory, not an archive, so `rsync` and `scp -r` already
    /// understand it. Move it off the machine; a backup on the same
    /// disk protects against nothing that has ever happened to a disk.
    Backup {
        /// Where to write it. Must not exist — a backup written over
        /// another one is two half-backups that look like one.
        #[arg(long, value_name = "PATH")]
        out: Option<std::path::PathBuf>,
    },

    /// Restore a database to a moment, as a new one beside it.
    ///
    /// **Never the original rewound.** Rewinding is irreversible and
    /// leaves the read-only copies ahead of their primary; this makes a
    /// copy, so the original goes on serving while somebody takes what
    /// they came for.
    ///
    /// It unpacks and replays at its next deployment. How far back it
    /// can reach is bounded by the oldest backup on the node, and how
    /// recent by the last archived segment — the database's own page
    /// says both.
    Restore {
        /// Which database, by the name the console shows.
        #[arg(value_name = "DATABASE")]
        database: String,

        /// The moment, in UTC: `2026-08-16 14:32`. Left out, it replays
        /// as far as the archived log goes.
        #[arg(long, value_name = "MOMENT")]
        to: Option<String>,

        /// What to call the copy. Defaults to the original's name with
        /// `-restored` after it.
        #[arg(long, value_name = "NAME")]
        into: Option<String>,

        /// A backup directory to restore from, when it is not one this
        /// node keeps — the one you moved off the machine and brought
        /// back, or one taken with `backup --out` somewhere else.
        #[arg(long, value_name = "PATH")]
        from: Option<std::path::PathBuf>,
    },

    /// Put a whole node back from a backup.
    ///
    /// Destructive: it replaces this node's database with the backup's,
    /// and the rows that are here now go with it. What was here is
    /// copied aside first, whatever happens.
    ///
    /// It will not guess whether this machine *is* the node that was
    /// backed up — see `--same-node` and `--new-node`, one of which is
    /// required.
    RestoreNode {
        /// The backup directory: `<root>/nodes/<node id>/<taken at>`.
        #[arg(long, value_name = "PATH")]
        from: std::path::PathBuf,

        /// This machine is that node. Its id, keys, grants and
        /// enrolments come back, and the network never notices the
        /// machine was replaced. This is rebuilding what died.
        #[arg(long, conflicts_with = "new_node")]
        same_node: bool,

        /// This machine takes the data and is somebody else. It mints a
        /// new identity and has to join again; the original node stays
        /// whatever it is.
        #[arg(long, conflicts_with = "same_node")]
        new_node: bool,

        /// Restore even though this node already has services of its
        /// own. They go with the database being replaced.
        #[arg(long)]
        over_my_dead_body: bool,
    },

    /// Talk to containerd and report what it says.
    ///
    /// Not part of running a node: it exists because the containerd
    /// client is written against generated bindings with no high-level
    /// API, and "does the connection work" deserves an answer that is
    /// not a deployment failing.
    Containerd {
        /// Pull this image and print what it says about itself.
        #[arg(long, value_name = "REF")]
        pull: Option<String>,

        /// Run this image as a container, report it, and remove it.
        ///
        /// The end-to-end check for the whole runtime path: pull,
        /// unpack, snapshot, spec, crun, task. Nothing survives it.
        #[arg(long, value_name = "REF")]
        run: Option<String>,

        /// What the container should listen on, handed to it as `PORT`.
        #[arg(long, value_name = "PORT", default_value_t = 8080)]
        port: u16,
    },
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

    /// Finish the install even if no public certificate was obtained.
    ///
    /// Without this, `install --domain` fails when the certificate
    /// does not arrive — because the alternative is an install that
    /// reports success and serves a certificate no browser trusts,
    /// which is discovered later by somebody who was not there.
    ///
    /// With it, the node serves its own authority's certificate and
    /// keeps asking in the background. That is a real way to run — a
    /// machine on a private network, or one whose DNS is still
    /// propagating — and it should be said out loud rather than fallen
    /// into.
    #[arg(long)]
    pub allow_self_signed: bool,

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
            // Tests that are not about the certificate must not fail
            // on one; the tests that *are* about it set this to false
            // and say so.
            allow_self_signed: true,
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

    /// The way back in takes a name, because a node has more than one
    /// account and a reset with no argument would be a coin toss about
    /// whose password just changed.
    #[test]
    fn passwd_takes_whose() {
        let cli = Cli::parse_from(["wabot-deploy", "passwd", "jorge"]);
        assert!(matches!(cli.command, Command::Passwd { username } if username == "jorge"));
        assert!(
            Cli::try_parse_from(["wabot-deploy", "passwd"]).is_err(),
            "and it will not guess"
        );
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

    /// The token is the whole of the command, and it is not optional:
    /// a bare `join` that did something would be a node joining
    /// whatever it last heard about.
    #[test]
    fn join_takes_a_token_and_needs_one() {
        let cli = Cli::parse_from(["wabot-deploy", "join", "wdj1.abc"]);
        match cli.command {
            Command::Join { token } => assert_eq!(token, "wdj1.abc"),
            other => panic!("expected join, got {other:?}"),
        }
        assert!(Cli::try_parse_from(["wabot-deploy", "join"]).is_err());
    }
}
