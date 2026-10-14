//! What the node is, as something that can change while it runs.
//!
//! The domain started as configuration: read from the file at boot,
//! carried around by value. That is fine until somebody has to change
//! it — and they do, because a node installed before its DNS was ready
//! serves a self-signed certificate and there is nothing to be done
//! about it from a console that only reads a value fixed at startup.
//!
//! So the database holds it and the file seeds it — **once**. [`seed`]
//! copies what the file and the environment say into the rows the first
//! time a node starts, and nothing reads the file for these again. It was
//! a fallback for as long as no row existed, and a fallback is a second
//! source of truth: the registry named every push after the value the
//! file had at boot, and a backup then decided which images were this
//! node's own by a different one. Found on a VM node.

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::{SqliteDatabase, SqliteResult};

/// Where the domain lives in the `setting` table.
const DOMAIN: &str = "node.domain";

/// Whether this node asks a public certificate authority for anything.
const ACME: &str = "acme.enabled";

/// Why the last certificate attempt failed.
///
/// A node-level fact rather than a column on the certificate row: the
/// interesting failure is the one where *no* certificate was obtained,
/// and that is exactly when there is no row to write it on. Recording
/// it beside the domain means the console can say why the name it was
/// given is not working.
const ACME_ERROR: &str = "acme.last_error";

/// What this node answers to, or `None` while nobody has said.
pub async fn domain(database: &SqliteDatabase) -> Option<String> {
    match read(database, DOMAIN).await {
        Ok(Some(stored)) if !stored.trim().is_empty() => Some(stored),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(%error, "could not read the node's domain");
            None
        }
    }
}

/// Set it, or clear it.
pub async fn set_domain(database: &SqliteDatabase, domain: Option<&str>) -> SqliteResult<()> {
    let value = domain.map(str::trim).unwrap_or_default().to_string();
    write(database, DOMAIN, &value).await
}

/// Whether a public authority is asked for certificates.
///
/// On when nothing is stored, which is what a node that has never been
/// seeded is: the production default, the same one `install` has always
/// had.
pub async fn acme_enabled(database: &SqliteDatabase) -> bool {
    !matches!(read(database, ACME).await, Ok(Some(value)) if value == "off")
}

pub async fn set_acme_enabled(database: &SqliteDatabase, on: bool) -> SqliteResult<()> {
    write(database, ACME, if on { "on" } else { "off" }).await
}

/// Copy what the file and the environment say into the rows, where no
/// row exists yet.
///
/// A row is written **whatever the file holds**, empty included, so the
/// file is read for these exactly once. Writing only a value left a node
/// installed without a domain consulting the file at every start, and an
/// operator who later typed one there would have found it applied — the
/// opposite of what the file's own header says.
pub async fn seed(
    database: &SqliteDatabase,
    config: &crate::infra::config::Config,
) -> SqliteResult<()> {
    if read(database, DOMAIN).await?.is_none() {
        set_domain(database, config.node.domain.as_deref()).await?;
    }
    if read(database, ACME).await?.is_none() {
        set_acme_enabled(database, !config.acme.disabled).await?;
    }
    Ok(())
}

/// Whether this node keeps the write-ahead log of its databases, so
/// that one can be restored to a point in time.
const ARCHIVING: &str = "wal.archiving";
const MEMORY_RESERVE: &str = "node.memory_reserve";
const CPU_RESERVE: &str = "node.cpu_reserve";

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
///
/// **And every node that existed before this got that explicit `off`**,
/// from `migrations/0040_archiving_default.sql`. The default is for
/// nodes being installed, where there is no database to disturb;
/// applying it to a running one would restart somebody's database
/// because a default changed, arriving with an upgrade they wanted for
/// something else.
pub async fn archiving(database: &SqliteDatabase) -> bool {
    !matches!(read(database, ARCHIVING).await, Ok(Some(value)) if value == "off")
}

/// Turn it on, or off.
pub async fn set_archiving(database: &SqliteDatabase, on: bool) -> SqliteResult<()> {
    write(database, ARCHIVING, if on { "on" } else { "off" }).await
}

/// What this node keeps for itself, in bytes, when the operator has said.
///
/// `None` means the default, which is a fraction *and* a floor — see
/// `presets::default_memory_reserve`. Stored rather than computed because
/// the default cannot be right for every machine: it is deliberately
/// generous on a small node, and on a 32 GB box 15 % is five gigabytes
/// nothing is using.
pub async fn memory_reserve(database: &SqliteDatabase) -> Option<u64> {
    match read(database, MEMORY_RESERVE).await {
        Ok(Some(value)) => value.trim().parse().ok(),
        _ => None,
    }
}

/// Set it, or clear it back to the default with `None`.
pub async fn set_memory_reserve(database: &SqliteDatabase, bytes: Option<u64>) -> SqliteResult<()> {
    write(
        database,
        MEMORY_RESERVE,
        &bytes.map(|bytes| bytes.to_string()).unwrap_or_default(),
    )
    .await
}

/// The same for CPU, in millicores.
pub async fn cpu_reserve(database: &SqliteDatabase) -> Option<u32> {
    match read(database, CPU_RESERVE).await {
        Ok(Some(value)) => value.trim().parse().ok(),
        _ => None,
    }
}

pub async fn set_cpu_reserve(
    database: &SqliteDatabase,
    millicores: Option<u32>,
) -> SqliteResult<()> {
    write(
        database,
        CPU_RESERVE,
        &millicores
            .map(|millicores| millicores.to_string())
            .unwrap_or_default(),
    )
    .await
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
                (key, value, crate::infra::clock::now_ms()),
            )?;
            Ok(())
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::Config;

    fn config_with(domain: Option<&str>) -> Config {
        let mut config = Config::default();
        config.node.domain = domain.map(str::to_string);
        config
    }

    #[tokio::test]
    async fn the_file_is_the_seed() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        seed(&database, &config_with(Some("from-the-file.example")))
            .await
            .expect("seed");

        assert_eq!(
            domain(&database).await.as_deref(),
            Some("from-the-file.example")
        );
    }

    /// A console change that a config file could overrule is a change
    /// that quietly stops applying at the next restart.
    #[tokio::test]
    async fn what_was_set_wins_over_the_file() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        set_domain(&database, Some("chosen.example"))
            .await
            .expect("set");

        seed(&database, &config_with(Some("from-the-file.example")))
            .await
            .expect("seed");
        assert_eq!(domain(&database).await.as_deref(), Some("chosen.example"));
    }

    /// Clearing it has to mean "no domain", not "fall back to the
    /// file" — otherwise removing a domain silently restores the one
    /// somebody was trying to leave behind.
    #[tokio::test]
    async fn clearing_it_means_none() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        set_domain(&database, None).await.expect("clear");

        seed(&database, &config_with(Some("from-the-file.example")))
            .await
            .expect("seed");
        assert_eq!(domain(&database).await, None);
    }

    /// The file is read once. A node installed with no domain wrote an
    /// empty row, so a name typed into the file afterwards is not one the
    /// node starts answering to at its next restart.
    #[tokio::test]
    async fn a_file_edited_after_the_first_start_changes_nothing() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        seed(&database, &config_with(None)).await.expect("first");

        let mut edited = config_with(Some("typed-later.example"));
        edited.acme.disabled = true;
        seed(&database, &edited).await.expect("second");

        assert_eq!(domain(&database).await, None);
        assert!(acme_enabled(&database).await);
    }

    /// `install --self-signed` and `WABOT_DEPLOY_ACME_DISABLED` arrive
    /// through the config, and have to land in the row that is read.
    #[tokio::test]
    async fn acme_switched_off_in_the_file_is_seeded_off() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let mut config = config_with(None);
        config.acme.disabled = true;

        seed(&database, &config).await.expect("seed");
        assert!(!acme_enabled(&database).await);

        set_acme_enabled(&database, true).await.expect("on");
        assert!(
            acme_enabled(&database).await,
            "and the console can turn it on"
        );
    }

    #[tokio::test]
    async fn a_node_with_nothing_anywhere_has_no_domain() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        assert_eq!(domain(&database).await, None);
    }

    /// Cleared has to read as "nothing wrong", not as an empty
    /// message the console would render as a blank failure line.
    #[tokio::test]
    async fn a_cleared_failure_is_no_failure() {
        let database = crate::infra::db::open_in_memory().await.expect("open");

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
        let database = crate::infra::db::open_in_memory().await.expect("open");
        set_domain(&database, Some("  spaced.example  "))
            .await
            .expect("set");

        assert_eq!(domain(&database).await.as_deref(), Some("spaced.example"));
    }
}
