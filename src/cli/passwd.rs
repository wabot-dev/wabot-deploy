//! `wabot-deploy passwd` — a way back into a node's own console.
//!
//! ## Why this exists
//!
//! A node whose only administrator forgot their password had none. The
//! setup token refuses to mint a second administrator — deliberately, that
//! is the whole of what makes it safe to print — and nothing else could
//! write a password. So the console was locked while the machine was
//! perfectly healthy, and the only route back was reinstalling.
//!
//! ## Why it is not a hole
//!
//! Whoever runs this already has root on the box: the binary, the
//! database, every container and every secret in them. A recovery path
//! root does not have is a lock on a door with no wall — what changes here
//! is only that recovery is obvious instead of surgery.
//!
//! ## Why it generates rather than asks
//!
//! A password given as an argument lands in shell history, in the process
//! table while it runs, and in whatever the terminal scrolled past. This
//! one is printed once, and what is stored is its hash.

use crate::config::Config;

pub async fn run(config: Config, username: &str) -> anyhow::Result<i32> {
    let database = crate::db::open(&config.database_path()).await?;
    let outcome = crate::accounts::reset_password(&database, username).await;
    database.close().await?;

    match outcome {
        Ok((name, password)) => {
            println!();
            println!("  {name}'s password is now:");
            println!();
            println!("  {password}");
            println!();
            println!(
                "  Sign in at {}, then change it if you want one you can",
                url(&config)
            );
            println!("  remember. Nothing else was touched.");
            println!();
            Ok(0)
        }
        Err(error) => {
            // Not a panic and not a stack trace: this runs on a terminal
            // and the answer is a sentence.
            println!("{error}");
            Ok(1)
        }
    }
}

/// Where to sign in, said the way `setup-token` says it.
fn url(config: &Config) -> String {
    match &config.node.domain {
        Some(domain) => format!("https://{domain}"),
        None => format!("https://<this node>:{}", config.edge.https_port),
    }
}
