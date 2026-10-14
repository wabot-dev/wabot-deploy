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

use crate::infra::config::Config;

pub async fn run(config: Config, username: &str) -> anyhow::Result<i32> {
    let database = crate::infra::db::open(&config.database_path()).await?;
    let outcome = crate::infra::accounts::reset_password(&database, username).await;
    let url = crate::cli::setup_token::url(&database, &config).await;
    database.close().await?;

    match outcome {
        Ok((name, password)) => {
            println!();
            println!("  {name}'s password is now:");
            println!();
            println!("  {password}");
            println!();
            // Not "change it once you are in": the console has no form
            // for that, and this said it did. Found on a VM node.
            println!("  Sign in at {url}, and keep it somewhere safe — there is no");
            println!("  form to change it, and this command issues another. Nothing");
            println!("  else was touched.");
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
