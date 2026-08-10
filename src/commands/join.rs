//! `wabot-deploy join <token>` — take instructions from another node.
//!
//! The sibling of `install`, and deliberately its own verb: installing
//! is what makes a machine a node, joining is what makes that node part
//! of somebody's network. Conflating them would mean the only way to
//! join is to reinstall.
//!
//! This is the terminal's door onto [`network::join`]; the console has
//! its own, and both do the same writes in the same order. What is here
//! and not there is the check that this machine is a node at all —
//! somebody looking at a console is already past that question.

use crate::config::Config;
use crate::network::{self, join::Joined};

pub async fn run(config: Config, token: &str) -> anyhow::Result<i32> {
    // Not `db::open`, which would create one: joining a machine that
    // has never been installed would leave a node with a grant, no
    // configuration and no service.
    if !config.database_path().exists() {
        println!("This machine is not a node yet.");
        println!();
        println!("  run `wabot-deploy install` first, then join.");
        return Ok(1);
    }
    let database = crate::db::open(&config.database_path()).await?;

    // `None`: running the command is the consent. There is nobody here
    // to show terms to, and somebody typing this has the token in their
    // hand — the console is where the screen belongs.
    let outcome = network::join::join(&database, &config, token, None).await;
    database.close().await?;

    match outcome {
        Ok(joined) => {
            report(&joined);
            Ok(0)
        }
        Err(error) => {
            println!("{error}");
            println!();
            println!("  Nothing was granted. Fix it and join again with the same");
            println!("  token — both ends treat a second attempt as the same join.");
            Ok(1)
        }
    }
}

fn report(joined: &Joined) {
    let Joined {
        authority,
        me,
        public_key,
    } = joined;

    println!("joined {} ({})", authority.name, authority.id);
    println!();
    println!("  this node   {} ({})", me.name, me.id);
    println!(
        "  overlay     {} on {}",
        me.overlay_ip.as_deref().unwrap_or("—"),
        network::overlay::SUBNET
    );
    println!("  public key  {public_key}");
    println!();
    println!(
        "  {} may now send this node errands. Nothing travels yet — the",
        authority.name
    );
    println!("  overlay is the next piece — and `wabot-deploy doctor` shows this");
    println!("  from here. To stop taking instructions, revoke it from the nodes");
    println!("  page of this node's own console.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::token::JoinToken;

    fn config_in(dir: &std::path::Path) -> Config {
        let mut config = Config::default();
        config.node.data_dir = dir.join("data");
        config.acme.disabled = true;
        config
    }

    fn token() -> JoinToken {
        JoinToken {
            authority: "nd-hub00000001".into(),
            name: "hub.example".into(),
            endpoint: "127.0.0.1:1".into(),
            public_key: "0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=".into(),
            overlay_ip: "10.42.0.1".into(),
            assigned_ip: "10.42.0.2".into(),
            secret: "a-very-long-secret".into(),
            // What a token minted before the terms existed carries, and
            // what it therefore means: both capabilities.
            requires: None,
            offers: None,
        }
    }

    /// A machine that is not a node has nowhere to put a grant, and
    /// creating the database here would leave one with no service.
    #[tokio::test]
    async fn joining_before_installing_says_so() {
        let dir = tempfile::tempdir().expect("tempdir");
        let code = run(config_in(dir.path()), &token().encode())
            .await
            .expect("ran");
        assert_eq!(code, 1);
        assert!(!config_in(dir.path()).database_path().exists());
    }

    /// A refusal is an exit code a script can read, not a panic.
    #[tokio::test]
    async fn a_join_that_does_not_land_exits_nonzero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = config_in(dir.path());
        crate::commands::install::run(
            config.clone(),
            &dir.path().join("config.toml"),
            crate::cli::InstallArgs::none(),
        )
        .await
        .expect("install");

        assert_eq!(run(config.clone(), "hunter2").await.expect("ran"), 1);
        assert_eq!(run(config, &token().encode()).await.expect("ran"), 1);
    }
}
