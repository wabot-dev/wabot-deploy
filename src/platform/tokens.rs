//! Push tokens: what a machine authenticates with.
//!
//! A CI job needs credentials, and giving it a person's is how a
//! password ends up in a config file on a build agent. A push token is
//! nobody's password: it belongs to a project, it can be revoked
//! without changing anything else, and the worst a leaked one does is
//! push images to that one project.
//!
//! Stored hashed, shown once. Same as every other token here.

use serde::Serialize;
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::{now_ms, PlatformError, PlatformResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushToken {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub created_at: i64,
    /// When something last authenticated with it — the answer to "is
    /// this one still in use" before somebody revokes it.
    pub last_used_at: Option<i64>,
}

/// Mint one, returning the secret in clear.
///
/// The only time it exists in clear. What is stored is its hash, which
/// is why no page can ever show it again.
pub async fn create(
    database: &SqliteDatabase,
    project_id: &str,
    name: &str,
    created_by: &str,
) -> PlatformResult<(PushToken, String)> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(PlatformError::Refused(
            "give the token a name — a list of five unnamed tokens is unreadable".into(),
        ));
    }

    let secret = wabot::prelude::password::generate(40);
    let token = PushToken {
        id: format!("pt-{}", wabot::prelude::password::generate(12)),
        project_id: project_id.to_string(),
        name: name.to_string(),
        created_at: now_ms(),
        last_used_at: None,
    };

    let row = token.clone();
    let hash = crate::accounts::sha256_hex(&secret);
    let creator = created_by.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO push_token \
                   (\"id\", \"project_id\", \"token_hash\", \"name\", \"created_by\", \
                    \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    row.id,
                    row.project_id,
                    hash,
                    row.name,
                    creator,
                    row.created_at,
                ),
            )?;
            Ok(())
        })
        .await?;

    Ok((token, secret))
}

/// Which project this token pushes to, if it is one.
///
/// Records the use as it goes: a token nobody has used in months is
/// the one to revoke, and that is only visible if using it says so.
pub async fn authenticate(
    database: &SqliteDatabase,
    secret: &str,
) -> PlatformResult<Option<String>> {
    let hash = crate::accounts::sha256_hex(secret);
    let lookup = hash.clone();

    let project: Option<String> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"project_id\" FROM push_token WHERE \"token_hash\" = ?1",
                    [lookup],
                    |row| row.get(0),
                )
                .optional()
        })
        .await?;

    if project.is_some() {
        database
            .write(move |connection| {
                connection.execute(
                    "UPDATE push_token SET \"last_used_at\" = ?2 WHERE \"token_hash\" = ?1",
                    (hash, now_ms()),
                )?;
                Ok(())
            })
            .await?;
    }
    Ok(project)
}

pub async fn of_project(
    database: &SqliteDatabase,
    project_id: &str,
) -> PlatformResult<Vec<PushToken>> {
    let id = project_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"project_id\", \"name\", \"created_at\", \"last_used_at\" \
                     FROM push_token WHERE \"project_id\" = ?1 ORDER BY \"created_at\" DESC",
                )?
                .query_map([id], |row| {
                    Ok(PushToken {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        name: row.get(2)?,
                        created_at: row.get(3)?,
                        last_used_at: row.get(4)?,
                    })
                })?
                .collect()
        })
        .await?)
}

pub async fn revoke(database: &SqliteDatabase, id: &str) -> PlatformResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM push_token WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::projects;

    /// A project and a real account to blame the token on — the
    /// foreign key is there so a token outlives nobody.
    async fn project() -> (SqliteDatabase, String, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let token = crate::accounts::issue_setup_token(&database)
            .await
            .expect("token");
        let admin = crate::accounts::create_admin(&database, &token, "admin", "a long passphrase")
            .await
            .expect("admin");
        let project = projects::create(&database, "demo").await.expect("project");
        (database, project.id, admin.id)
    }

    #[tokio::test]
    async fn a_token_authenticates_for_its_project() {
        let (database, project, admin) = project().await;
        let (_, secret) = create(&database, &project, "ci", &admin)
            .await
            .expect("created");

        assert_eq!(
            authenticate(&database, &secret).await.expect("auth"),
            Some(project)
        );
    }

    #[tokio::test]
    async fn something_that_is_not_a_token_is_nobody() {
        let (database, project, admin) = project().await;
        create(&database, &project, "ci", &admin)
            .await
            .expect("created");

        assert_eq!(
            authenticate(&database, "made-up").await.expect("auth"),
            None
        );
    }

    /// A token for one project must not push to another. This is the
    /// whole reason it is scoped rather than being a node credential.
    #[tokio::test]
    async fn a_token_belongs_to_one_project() {
        let (database, first, admin) = project().await;
        let second = projects::create(&database, "other")
            .await
            .expect("project")
            .id;

        let (_, secret) = create(&database, &first, "ci", &admin)
            .await
            .expect("created");
        let holder = authenticate(&database, &secret).await.expect("auth");

        assert_eq!(holder.as_deref(), Some(first.as_str()));
        assert_ne!(holder.as_deref(), Some(second.as_str()));
    }

    /// A token nobody has used is the one to revoke, and that is only
    /// visible if using it records something.
    #[tokio::test]
    async fn using_a_token_is_recorded() {
        let (database, project, admin) = project().await;
        let (token, secret) = create(&database, &project, "ci", &admin)
            .await
            .expect("created");
        assert_eq!(token.last_used_at, None);

        authenticate(&database, &secret).await.expect("auth");

        let after = of_project(&database, &project)
            .await
            .expect("list")
            .pop()
            .expect("one");
        assert!(after.last_used_at.is_some());
    }

    #[tokio::test]
    async fn a_revoked_token_stops_working() {
        let (database, project, admin) = project().await;
        let (token, secret) = create(&database, &project, "ci", &admin)
            .await
            .expect("created");

        revoke(&database, &token.id).await.expect("revoked");
        assert_eq!(authenticate(&database, &secret).await.expect("auth"), None);
    }

    /// A database somebody reads must not be a database somebody
    /// pushes with.
    #[tokio::test]
    async fn the_secret_is_not_stored() {
        let (database, project, admin) = project().await;
        let (_, secret) = create(&database, &project, "ci", &admin)
            .await
            .expect("created");

        let stored: String = database
            .read(|connection| {
                connection.query_row("SELECT \"token_hash\" FROM push_token", [], |row| {
                    row.get(0)
                })
            })
            .await
            .expect("query");
        assert_ne!(stored, secret);
        assert_eq!(stored, crate::accounts::sha256_hex(&secret));
    }

    #[tokio::test]
    async fn a_token_needs_a_name() {
        let (database, project, admin) = project().await;
        assert!(create(&database, &project, "  ", &admin).await.is_err());
    }

    #[tokio::test]
    async fn tokens_go_with_the_project() {
        let (database, project, admin) = project().await;
        create(&database, &project, "ci", &admin)
            .await
            .expect("created");

        projects::delete(&database, &project).await.expect("delete");
        assert!(of_project(&database, &project)
            .await
            .expect("list")
            .is_empty());
    }
}
