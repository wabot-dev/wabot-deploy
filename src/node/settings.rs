//! What the node is, as something that can change while it runs.
//!
//! The domain started as configuration: read from the file at boot,
//! carried around by value. That is fine until somebody has to change
//! it — and they do, because a node installed before its DNS was ready
//! serves a self-signed certificate and there is nothing to be done
//! about it from a console that only reads a value fixed at startup.
//!
//! So the database holds it and the file seeds it. The file stays
//! useful — it is what an operator edits before the first boot, and
//! what an install writes — but from then on the answer to "what is
//! this node called" comes from one place that a request can change.

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::{SqliteDatabase, SqliteResult};

/// Where the domain lives in the `setting` table.
const DOMAIN: &str = "node.domain";

/// Why the last certificate attempt failed.
///
/// A node-level fact rather than a column on the certificate row: the
/// interesting failure is the one where *no* certificate was obtained,
/// and that is exactly when there is no row to write it on. Recording
/// it beside the domain means the console can say why the name it was
/// given is not working.
const ACME_ERROR: &str = "acme.last_error";

/// What this node answers to, or `None` while nobody has said.
///
/// The stored value wins over the file. A console that could be
/// overruled by a config file nobody edited would be a console whose
/// changes quietly stop applying at the next restart.
pub async fn domain(database: &SqliteDatabase, config: &crate::config::Config) -> Option<String> {
    match read(database, DOMAIN).await {
        Ok(Some(stored)) if !stored.trim().is_empty() => Some(stored),
        // Nothing stored: the file is the seed. An explicit empty
        // value stored by the console means "no domain", and has to
        // survive being read back — hence the emptiness check above
        // rather than treating empty as absent.
        Ok(Some(_)) => None,
        Ok(None) => config.node.domain.clone(),
        Err(error) => {
            tracing::warn!(%error, "could not read the node's domain; using the config file");
            config.node.domain.clone()
        }
    }
}

/// Set it, or clear it.
pub async fn set_domain(database: &SqliteDatabase, domain: Option<&str>) -> SqliteResult<()> {
    let value = domain.map(str::trim).unwrap_or_default().to_string();
    write(database, DOMAIN, &value).await
}

/// Whether this node keeps the write-ahead log of its databases, so
/// that one can be restored to a point in time.
const ARCHIVING: &str = "wal.archiving";

/// Whether point-in-time recovery is on for this node.
///
/// **On by default, now that there is pruning.** It was off while there
/// was none, because Postgres will not delete a segment it has not
/// archived — so an archive nothing bounded was a slow disk leak with
/// the database's own life attached to it, and the day the disk filled
/// was the day the database stopped.
///
/// With `backup::sweep` running hourly the archive is bounded by the
/// recovery window, and the feature is worth having on: restoring a
/// database to the minute before somebody dropped a table is most of
/// what makes a managed database worth using, and a feature that only
/// works for the operators who found the switch is a feature most people
/// discover they did not have on the day they need it.
///
/// An explicit `off` still wins. A node with little disk, or one whose
/// databases hold nothing anybody would miss, is a real answer.
pub async fn archiving(database: &SqliteDatabase) -> bool {
    !matches!(read(database, ARCHIVING).await, Ok(Some(value)) if value == "off")
}

/// Turn it on, or off.
pub async fn set_archiving(database: &SqliteDatabase, on: bool) -> SqliteResult<()> {
    write(database, ARCHIVING, if on { "on" } else { "off" }).await
}

/// What the last certificate attempt said, if it failed.
pub async fn acme_error(database: &SqliteDatabase) -> Option<String> {
    match read(database, ACME_ERROR).await {
        Ok(stored) => stored.filter(|message| !message.trim().is_empty()),
        Err(error) => {
            tracing::warn!(%error, "could not read the last certificate failure");
            None
        }
    }
}

/// Record a failure, or clear it once a certificate arrives.
pub async fn set_acme_error(database: &SqliteDatabase, message: Option<&str>) -> SqliteResult<()> {
    write(database, ACME_ERROR, message.unwrap_or_default()).await
}

/// The same table, for a caller that owns its own key.
///
/// `pub(crate)` rather than another pair of named accessors here: the
/// `setting` table is the node's key-value store and this module is
/// where opening it lives, but what `network.private_key` means is
/// `network::keys`'s business, not this module's.
pub(crate) async fn read(database: &SqliteDatabase, key: &str) -> SqliteResult<Option<String>> {
    let key = key.to_string();
    database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"value\" FROM setting WHERE \"key\" = ?1",
                    [key],
                    |row| row.get(0),
                )
                .optional()
        })
        .await
}

pub(crate) async fn write(database: &SqliteDatabase, key: &str, value: &str) -> SqliteResult<()> {
    let (key, value) = (key.to_string(), value.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO setting (\"key\", \"value\", \"updated_at\") \
                 VALUES (?1, ?2, ?3) \
                 ON CONFLICT (\"key\") DO UPDATE SET \
                   \"value\" = excluded.\"value\", \
                   \"updated_at\" = excluded.\"updated_at\"",
                (key, value, crate::platform::now_ms()),
            )?;
            Ok(())
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn config_with(domain: Option<&str>) -> Config {
        let mut config = Config::default();
        config.node.domain = domain.map(str::to_string);
        config
    }

    #[tokio::test]
    async fn the_file_is_the_seed() {
        let database = crate::db::open_in_memory().await.expect("open");
        let config = config_with(Some("from-the-file.example"));

        assert_eq!(
            domain(&database, &config).await.as_deref(),
            Some("from-the-file.example")
        );
    }

    /// A console change that a config file could overrule is a change
    /// that quietly stops applying at the next restart.
    #[tokio::test]
    async fn what_was_set_wins_over_the_file() {
        let database = crate::db::open_in_memory().await.expect("open");
        let config = config_with(Some("from-the-file.example"));

        set_domain(&database, Some("chosen.example"))
            .await
            .expect("set");
        assert_eq!(
            domain(&database, &config).await.as_deref(),
            Some("chosen.example")
        );
    }

    /// Clearing it has to mean "no domain", not "fall back to the
    /// file" — otherwise removing a domain silently restores the one
    /// somebody was trying to leave behind.
    #[tokio::test]
    async fn clearing_it_means_none() {
        let database = crate::db::open_in_memory().await.expect("open");
        let config = config_with(Some("from-the-file.example"));

        set_domain(&database, None).await.expect("clear");
        assert_eq!(domain(&database, &config).await, None);
    }

    #[tokio::test]
    async fn a_node_with_nothing_anywhere_has_no_domain() {
        let database = crate::db::open_in_memory().await.expect("open");
        assert_eq!(domain(&database, &config_with(None)).await, None);
    }

    /// Cleared has to read as "nothing wrong", not as an empty
    /// message the console would render as a blank failure line.
    #[tokio::test]
    async fn a_cleared_failure_is_no_failure() {
        let database = crate::db::open_in_memory().await.expect("open");

        set_acme_error(&database, Some("dns did not resolve"))
            .await
            .expect("record");
        assert_eq!(
            acme_error(&database).await.as_deref(),
            Some("dns did not resolve")
        );

        set_acme_error(&database, None).await.expect("clear");
        assert_eq!(acme_error(&database).await, None);
    }

    #[tokio::test]
    async fn it_is_trimmed_on_the_way_in() {
        let database = crate::db::open_in_memory().await.expect("open");
        set_domain(&database, Some("  spaced.example  "))
            .await
            .expect("set");

        assert_eq!(
            domain(&database, &config_with(None)).await.as_deref(),
            Some("spaced.example")
        );
    }
}
