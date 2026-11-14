//! `wabot-deploy allowlist` — who may connect through the node's own
//! doors, from the machine's own terminal.
//!
//! ## Why this exists
//!
//! A list can shut out the person who wrote it. The console refuses one
//! that leaves out the address it is saved from, but an office whose
//! address changes next week is not something a form can see coming, and
//! SSH closed from the console leaves the console as the only way in. When
//! that goes too, what is left is a terminal on the machine — the
//! provider's serial console — and whoever has one has root already. This
//! is the way back that does not need the console.
//!
//! ## Why the node does the rest
//!
//! It writes the rows. SSH's rules are written here as well, because the
//! person running this may be waiting at an SSH prompt; the console's and
//! the registry's lists are the running node's, which reads them again
//! within ten seconds.

use clap::ValueEnum;

use crate::domain::values::allowlist::{Allowlist, Door};
use crate::infra::config::Config;
use crate::infra::repositories::allowlists;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Which {
    Console,
    Registry,
    Ssh,
}

impl Which {
    fn door(self) -> Door {
        match self {
            Which::Console => Door::Console,
            Which::Registry => Door::Registry,
            Which::Ssh => Door::Ssh,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Which::Console => "console",
            Which::Registry => "registry",
            Which::Ssh => "ssh",
        }
    }
}

pub async fn run(
    config: Config,
    which: Option<Which>,
    addresses: &[String],
) -> anyhow::Result<i32> {
    let database = crate::infra::db::open(&config.database_path()).await?;
    let outcome = act(&database, which, addresses).await;
    database.close().await?;
    match outcome {
        Ok(()) => Ok(0),
        Err(reason) => {
            eprintln!("wabot-deploy: {reason}");
            Ok(1)
        }
    }
}

async fn act(
    database: &wabot::sqlite::SqliteDatabase,
    which: Option<Which>,
    addresses: &[String],
) -> Result<(), String> {
    let shown = match which {
        Some(which) => vec![which],
        None => vec![Which::Console, Which::Registry, Which::Ssh],
    };
    let Some(which) = which.filter(|_| !addresses.is_empty()) else {
        println!();
        for which in shown {
            let list = allowlists::of(database, &which.door())
                .await
                .map_err(|error| error.to_string())?;
            println!("  {:<9} {}", which.name(), summary(&list));
        }
        println!();
        return Ok(());
    };

    let list = asked(addresses)?;
    allowlists::set(database, &which.door(), &list)
        .await
        .map_err(|error| error.to_string())?;
    println!();
    println!("  {:<9} {}", which.name(), summary(&list));
    println!();
    match which {
        Which::Ssh => {
            crate::infra::firewall::arm();
            crate::infra::firewall::heal(database).await;
            match crate::infra::firewall::last_error(database).await {
                None => println!("  In force now, for new connections."),
                Some(reason) => return Err(format!("saved, and not in force: {reason}")),
            }
        }
        Which::Console | Which::Registry => {
            println!("  The node reads it within ten seconds.");
        }
    }
    println!();
    Ok(())
}

/// What was typed, as a list. `everyone` and `nobody` are words for the
/// two lists people mean most, so the one somebody types at a recovery
/// prompt is not two CIDR ranges from memory.
fn asked(addresses: &[String]) -> Result<Allowlist, String> {
    if addresses.len() == 1 && addresses[0] == "nobody" {
        return Ok(Allowlist::default());
    }
    if addresses.iter().any(|address| address == "nobody") {
        return Err("`nobody` is a list on its own: it cannot be added to".into());
    }
    let lines: Vec<String> = addresses
        .iter()
        .map(|address| match address.as_str() {
            "everyone" => Allowlist::everyone().text(),
            other => other.to_string(),
        })
        .collect();
    Allowlist::parse(&lines.join("\n"))
}

pub(crate) fn summary(list: &Allowlist) -> String {
    if list.is_empty() {
        return "nobody".into();
    }
    if list.is_everyone() {
        return "everyone".into();
    }
    list.text().replace('\n', ", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(words: &[&str]) -> Result<Allowlist, String> {
        asked(
            &words
                .iter()
                .map(|word| word.to_string())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn the_two_words_are_the_two_lists_people_mean() {
        assert!(typed(&["everyone"]).expect("parsed").is_everyone());
        assert!(typed(&["nobody"]).expect("parsed").is_empty());
        assert!(typed(&["nobody", "203.0.113.5"]).is_err());
    }

    #[test]
    fn each_argument_is_a_line_and_may_carry_its_note() {
        let list = typed(&["203.0.113.5 office", "198.51.100.0/24"]).expect("parsed");
        assert_eq!(list.text(), "203.0.113.5 office\n198.51.100.0/24");
        assert!(typed(&["203.0.113.500"]).is_err());
    }

    #[tokio::test]
    async fn a_recovery_writes_the_list_the_console_reads() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        act(&database, Some(Which::Console), &["198.51.100.7".into()])
            .await
            .expect("written");
        assert_eq!(
            allowlists::of(&database, &Door::Console)
                .await
                .expect("read")
                .text(),
            "198.51.100.7"
        );
    }
}
