//! Releases: which image a service ran, and which it runs now.
//!
//! ## A release is a digest, not a tag
//!
//! Tags move. "Roll back to yesterday's `latest`" means nothing when
//! yesterday's `latest` is today's, and a rollback that redeploys a
//! tag is a rollback that reruns the thing being rolled back from.
//! Every deployment pins the digest.
//!
//! ## The current one is marked, not derived
//!
//! "The newest" is not the answer: a rollback makes an *older* release
//! the current one, which is the entire feature.

use serde::Serialize;
use wabot::sqlite::SqliteDatabase;

use super::{now_ms, PlatformResult};

/// Where a release came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The registry received it.
    Push,
    /// Somebody pointed the service at an image.
    Manual,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Push => "push",
            Source::Manual => "manual",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "push" => Source::Push,
            _ => Source::Manual,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Source::Push => "Pushed",
            Source::Manual => "Set by hand",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Release {
    pub id: String,
    pub service_id: String,
    pub reference: String,
    pub digest: String,
    pub source: Source,
    pub created_at: i64,
    /// When this one was last deployed, if it ever was.
    pub deployed_at: Option<i64>,
}

impl Release {
    /// The tag this was pushed as: `latest`, `v2`, `2026-08-19`.
    ///
    /// What tells two releases of one service apart at a glance — the
    /// repository is the same on every one of them, and the digest is
    /// eight hex characters nobody can hold in their head. Falls back to
    /// the whole reference when there is no tag to take, which is a
    /// digest-pinned reference and reads as itself.
    pub fn tag(&self) -> &str {
        // From the last colon, and only if it is after the last slash: a
        // registry may carry a port, and `node:5000/app` has a colon that
        // is not a tag's.
        match self.reference.rsplit_once(':') {
            Some((before, tag)) if !before.ends_with('/') && !tag.contains('/') => tag,
            _ => &self.reference,
        }
    }

    /// The digest as somebody would read it: `sha256:1a2b3c4d`.
    ///
    /// Enough to tell two builds apart at a glance, and the full value
    /// is one hover away in the title attribute.
    pub fn short_digest(&self) -> String {
        match self.digest.split_once(':') {
            Some((algorithm, hex)) => format!("{algorithm}:{}", &hex[..hex.len().min(12)]),
            None => self.digest.clone(),
        }
    }

    /// What to run: the repository at this exact digest.
    ///
    /// Not the tag it arrived under — that is a label on a moving
    /// target, and the point of a release is that it does not move.
    pub fn pinned(&self) -> String {
        let repository = super::images::Reference::parse(&self.reference)
            .map(|reference| reference.repository)
            .unwrap_or_else(|| self.reference.clone());
        format!("{repository}@{}", self.digest)
    }
}

/// Record an image for a service.
///
/// Idempotent on (service, reference, digest): pushing the same bytes
/// under the same tag twice is one release, and CI that retries must
/// not fill the history with copies.
pub async fn record(
    database: &SqliteDatabase,
    service_id: &str,
    reference: &str,
    digest: &str,
    source: Source,
) -> PlatformResult<Release> {
    let release = Release {
        id: format!("rel-{}", wabot::prelude::password::generate(12)),
        service_id: service_id.to_string(),
        reference: reference.to_string(),
        digest: digest.to_string(),
        source,
        created_at: now_ms(),
        deployed_at: None,
    };

    let row = release.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO release \
                   (\"id\", \"service_id\", \"reference\", \"digest\", \"source\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT (\"service_id\", \"reference\", \"digest\") DO NOTHING",
                (
                    row.id,
                    row.service_id,
                    row.reference,
                    row.digest,
                    row.source.as_str(),
                    row.created_at,
                ),
            )?;
            Ok(())
        })
        .await?;

    // Read it back rather than returning what was built: on a repeat
    // push the row that matters is the one already there, with its
    // original id and timestamp.
    find_by_digest(database, service_id, reference, digest)
        .await?
        .ok_or_else(|| {
            super::PlatformError::Refused("the release vanished as it was written".into())
        })
}

/// Mark this release as the one running, and no other.
pub async fn mark_deployed(database: &SqliteDatabase, release_id: &str) -> PlatformResult<()> {
    let id = release_id.to_string();
    database
        .write(move |connection| {
            // Scoped to the service, so deploying here does not clear
            // what another service is running.
            connection.execute(
                "UPDATE release SET \"deployed_at\" = NULL \
                 WHERE \"service_id\" = (SELECT \"service_id\" FROM release WHERE \"id\" = ?1)",
                [&id],
            )?;
            connection.execute(
                "UPDATE release SET \"deployed_at\" = ?2 WHERE \"id\" = ?1",
                (&id, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Every release for a service, newest first.
pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Vec<Release>> {
    let id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"service_id\", \"reference\", \"digest\", \"source\", \
                     \"created_at\", \"deployed_at\" FROM release \
                     WHERE \"service_id\" = ?1 \
                     ORDER BY \"created_at\" DESC, \"rowid\" DESC",
                )?
                .query_map([id], decode)?
                .collect()
        })
        .await?)
}

pub async fn find(database: &SqliteDatabase, id: &str) -> PlatformResult<Option<Release>> {
    let id = id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"service_id\", \"reference\", \"digest\", \"source\", \
                     \"created_at\", \"deployed_at\" FROM release WHERE \"id\" = ?1",
                )?
                .query_map([id], decode)?
                .collect::<Result<Vec<_>, _>>()
        })
        .await?
        .into_iter()
        .next())
}

async fn find_by_digest(
    database: &SqliteDatabase,
    service_id: &str,
    reference: &str,
    digest: &str,
) -> PlatformResult<Option<Release>> {
    let (service_id, reference, digest) = (
        service_id.to_string(),
        reference.to_string(),
        digest.to_string(),
    );
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"service_id\", \"reference\", \"digest\", \"source\", \
                     \"created_at\", \"deployed_at\" FROM release \
                     WHERE \"service_id\" = ?1 AND \"reference\" = ?2 AND \"digest\" = ?3",
                )?
                .query_map((service_id, reference, digest), decode)?
                .collect::<Result<Vec<_>, _>>()
        })
        .await?
        .into_iter()
        .next())
}

fn decode(row: &wabot::sqlite::rusqlite::Row<'_>) -> wabot::sqlite::rusqlite::Result<Release> {
    Ok(Release {
        id: row.get(0)?,
        service_id: row.get(1)?,
        reference: row.get(2)?,
        digest: row.get(3)?,
        source: Source::parse(&row.get::<_, String>(4)?),
        created_at: row.get(5)?,
        deployed_at: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    fn released(reference: &str) -> Release {
        Release {
            id: "rl-1".into(),
            service_id: "sv-1".into(),
            reference: reference.into(),
            digest: "sha256:1a2b3c4d5e6f".into(),
            source: Source::Push,
            created_at: 0,
            deployed_at: None,
        }
    }

    /// The tag is what tells two releases of one service apart.
    ///
    /// The list showed only eight hex characters of digest under a column
    /// headed "Image", which distinguishes nothing — the repository is
    /// identical on every row, and the digest is not something anybody
    /// holds in their head. Reported by Jorge, who wanted to see which
    /// image he had pushed.
    #[test]
    fn a_release_is_known_by_its_tag() {
        assert_eq!(released("node.example/wabot/api:latest").tag(), "latest");
        assert_eq!(released("node.example/wabot/api:v2").tag(), "v2");
    }

    /// A registry's port is not a tag.
    ///
    /// `node:5000/app` has a colon before the last slash, and taking the
    /// text after the last colon would call the tag `5000/app`.
    #[test]
    fn a_port_in_the_registry_is_not_mistaken_for_one() {
        assert_eq!(released("node:5000/wabot/api:v2").tag(), "v2");
        // And with no tag at all, the reference reads as itself rather
        // than as a tag that is not there.
        let pinned = released("node.example/wabot/api");
        assert_eq!(pinned.tag(), "node.example/wabot/api");
    }

    use super::*;
    use crate::platform::{projects, services};

    /// The one marked as running, which is what every page shows —
    /// read from the list rather than from a query of its own, because
    /// the list is what a page already has.
    async fn deployed(database: &SqliteDatabase, service_id: &str) -> Option<Release> {
        of_service(database, service_id)
            .await
            .expect("list")
            .into_iter()
            .find(|release| release.deployed_at.is_some())
    }

    async fn service() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "demo").await.expect("project");
        let service = services::create(
            &database,
            &project.id,
            "api",
            "node.example/demo/api:latest",
            &[],
        )
        .await
        .expect("service");
        (database, service.id)
    }

    #[tokio::test]
    async fn a_release_records_what_was_pushed() {
        let (database, service) = service().await;
        let release = record(
            &database,
            &service,
            "node.example/demo/api:latest",
            "sha256:aaaa",
            Source::Push,
        )
        .await
        .expect("recorded");

        assert_eq!(release.source, Source::Push);
        assert_eq!(release.deployed_at, None, "recorded is not deployed");
        assert_eq!(
            of_service(&database, &service).await.expect("list").len(),
            1
        );
    }

    /// CI that retries must not fill the history with copies of one
    /// build.
    #[tokio::test]
    async fn the_same_image_pushed_twice_is_one_release() {
        let (database, service) = service().await;
        let first = record(
            &database,
            &service,
            "node.example/demo/api:latest",
            "sha256:aaaa",
            Source::Push,
        )
        .await
        .expect("recorded");
        let again = record(
            &database,
            &service,
            "node.example/demo/api:latest",
            "sha256:aaaa",
            Source::Push,
        )
        .await
        .expect("recorded");

        assert_eq!(first.id, again.id, "the same row came back");
        assert_eq!(
            of_service(&database, &service).await.expect("list").len(),
            1
        );
    }

    /// A tag that moved is a new release, because the bytes are
    /// different — which is exactly what a release is for.
    #[tokio::test]
    async fn the_same_tag_with_new_bytes_is_a_new_release() {
        let (database, service) = service().await;
        for digest in ["sha256:aaaa", "sha256:bbbb"] {
            record(
                &database,
                &service,
                "node.example/demo/api:latest",
                digest,
                Source::Push,
            )
            .await
            .expect("recorded");
        }
        assert_eq!(
            of_service(&database, &service).await.expect("list").len(),
            2
        );
    }

    /// The whole point: an older release becomes the current one, and
    /// "current" cannot therefore mean "newest".
    #[tokio::test]
    async fn rolling_back_makes_an_older_release_current() {
        let (database, service) = service().await;
        let old = record(&database, &service, "r:1", "sha256:aaaa", Source::Push)
            .await
            .expect("recorded");
        let new = record(&database, &service, "r:2", "sha256:bbbb", Source::Push)
            .await
            .expect("recorded");

        mark_deployed(&database, &new.id).await.expect("deployed");
        assert_eq!(
            deployed(&database, &service).await.map(|r| r.id),
            Some(new.id.clone())
        );

        mark_deployed(&database, &old.id)
            .await
            .expect("rolled back");
        let current = deployed(&database, &service).await.expect("one");
        assert_eq!(current.id, old.id);
        assert!(
            current.created_at
                <= find(&database, &new.id)
                    .await
                    .expect("find")
                    .expect("present")
                    .created_at,
            "the current one is the older one"
        );
    }

    /// Two releases marked as running would make "what is deployed" a
    /// question with two answers.
    #[tokio::test]
    async fn only_one_release_is_current_at_a_time() {
        let (database, service) = service().await;
        let first = record(&database, &service, "r:1", "sha256:aaaa", Source::Push)
            .await
            .expect("recorded");
        let second = record(&database, &service, "r:2", "sha256:bbbb", Source::Push)
            .await
            .expect("recorded");

        mark_deployed(&database, &first.id).await.expect("deployed");
        mark_deployed(&database, &second.id)
            .await
            .expect("deployed");

        let deployed: Vec<_> = of_service(&database, &service)
            .await
            .expect("list")
            .into_iter()
            .filter(|release| release.deployed_at.is_some())
            .collect();
        assert_eq!(deployed.len(), 1);
        assert_eq!(deployed[0].id, second.id);
    }

    /// What actually gets run: the repository at a digest, never the
    /// tag it arrived under.
    #[tokio::test]
    async fn a_release_runs_by_digest() {
        let (database, service) = service().await;
        let release = record(
            &database,
            &service,
            "node.example/demo/api:latest",
            "sha256:abcdef0123456789",
            Source::Push,
        )
        .await
        .expect("recorded");

        assert_eq!(
            release.pinned(),
            "node.example/demo/api@sha256:abcdef0123456789"
        );
        assert_eq!(release.short_digest(), "sha256:abcdef012345");
    }

    #[tokio::test]
    async fn releases_go_with_the_service() {
        let (database, service) = service().await;
        record(&database, &service, "r:1", "sha256:aaaa", Source::Push)
            .await
            .expect("recorded");

        services::delete(&database, &service).await.expect("delete");
        assert!(of_service(&database, &service)
            .await
            .expect("list")
            .is_empty());
    }
}
