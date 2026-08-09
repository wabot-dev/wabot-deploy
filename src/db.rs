//! Opening the node's database, and the two tables it starts with.
//!
//! Migrations are `include_str!`ed rather than read from disk: the
//! binary is copied to a machine and run there, and a daemon that
//! needed a sibling `migrations/` directory would not be the single
//! file the product claims to be.

use std::path::Path;

use wabot::sqlite::{Migration, MigrationRunner, SqliteConfig, SqliteDatabase, SqliteResult};

/// Applied in id order, once each, checksummed. Add to the end.
fn migrations() -> Vec<Migration> {
    vec![
        Migration::new("0001_node", include_str!("../migrations/0001_node.sql")),
        Migration::new("0002_edge", include_str!("../migrations/0002_edge.sql")),
        Migration::new("0003_acme", include_str!("../migrations/0003_acme.sql")),
        Migration::new(
            "0004_accounts",
            include_str!("../migrations/0004_accounts.sql"),
        ),
        Migration::new(
            "0005_projects",
            include_str!("../migrations/0005_projects.sql"),
        ),
        Migration::new(
            "0006_network",
            include_str!("../migrations/0006_network.sql"),
        ),
        Migration::new("0007_ports", include_str!("../migrations/0007_ports.sql")),
        Migration::new("0008_people", include_str!("../migrations/0008_people.sql")),
        Migration::new(
            "0009_registry",
            include_str!("../migrations/0009_registry.sql"),
        ),
        Migration::new(
            "0010_registry_tags",
            include_str!("../migrations/0010_registry_tags.sql"),
        ),
        Migration::new(
            "0011_updates",
            include_str!("../migrations/0011_updates.sql"),
        ),
        Migration::new(
            "0012_certificate_sources",
            include_str!("../migrations/0012_certificate_sources.sql"),
        ),
        Migration::new(
            "0013_certificate_failures",
            include_str!("../migrations/0013_certificate_failures.sql"),
        ),
        Migration::new(
            "0014_account_theme",
            include_str!("../migrations/0014_account_theme.sql"),
        ),
        Migration::new(
            "0015_network",
            include_str!("../migrations/0015_network.sql"),
        ),
    ]
}

/// Open `path` and bring the schema up to date.
pub async fn open(path: &Path) -> SqliteResult<SqliteDatabase> {
    let database = SqliteDatabase::open(SqliteConfig::at(path))?;
    migrate(&database).await?;
    Ok(database)
}

/// A database with no file behind it: same schema, same SQL, nothing
/// on disk.
///
/// Test-only, and worth having rather than reaching for
/// `InMemoryRepository`: this exercises the real migrations and the
/// real SQL, so a query that renders wrongly fails here instead of on
/// a node.
#[cfg(test)]
pub async fn open_in_memory() -> SqliteResult<SqliteDatabase> {
    let database = SqliteDatabase::in_memory()?;
    migrate(&database).await?;
    Ok(database)
}

pub async fn migrate(database: &SqliteDatabase) -> SqliteResult<Vec<String>> {
    let applied = runner(database).up().await?;
    if !applied.is_empty() {
        tracing::info!(migrations = applied.join(", "), "schema updated");
    }
    Ok(applied)
}

/// What is applied and what is pending — `doctor` prints it.
pub async fn status(
    database: &SqliteDatabase,
) -> SqliteResult<Vec<wabot::sqlite::MigrationStatus>> {
    runner(database).status().await
}

fn runner(database: &SqliteDatabase) -> MigrationRunner {
    MigrationRunner::new(database.clone()).with_migrations(migrations())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_schema_applies_and_is_idempotent() {
        let database = open_in_memory().await.expect("open");

        let status = status(&database).await.expect("status");
        assert!(
            status.iter().all(|s| s.applied && !s.drifted),
            "every migration applied cleanly: {status:?}"
        );

        let count: i64 = database
            .read(|c| {
                c.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='node_state'",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("query");
        assert_eq!(count, 1, "node_state is missing");

        assert!(
            migrate(&database).await.expect("migrate").is_empty(),
            "a second run has nothing to do"
        );
    }

    /// The path `install` takes: a real file, created along with the
    /// directories above it, and reopened without re-migrating.
    #[tokio::test]
    async fn opening_a_file_creates_the_directories_and_survives_a_reopen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("data").join("db").join("node.db");

        let database = open(&path).await.expect("open");
        crate::ledger::record(
            &database,
            crate::ledger::Step::Layout,
            crate::ledger::Status::Done,
            None,
        )
        .await
        .expect("record");
        database.close().await.expect("close");

        assert!(path.exists(), "the database file was created");

        let database = open(&path).await.expect("reopen");
        assert!(
            crate::ledger::is_done(&database, crate::ledger::Step::Layout)
                .await
                .expect("is_done"),
            "the row written before the close is still there"
        );
    }
}
