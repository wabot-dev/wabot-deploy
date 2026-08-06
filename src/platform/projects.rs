//! Projects: a workspace, and the services under it.

use serde::Serialize;
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::{now_ms, slugify, PlatformError, PlatformResult};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub created_at: i64,
}

pub async fn create(database: &SqliteDatabase, name: &str) -> PlatformResult<Project> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(PlatformError::Refused(
            "a project name is between 1 and 60 characters".into(),
        ));
    }

    let slug = slugify(name);
    if slug.is_empty() {
        return Err(PlatformError::Refused(
            "that name has nothing a hostname can be built from — use letters or digits".into(),
        ));
    }

    let project = Project {
        id: format!("prj-{}", wabot::prelude::password::generate(12)),
        name: name.to_string(),
        slug,
        created_at: now_ms(),
    };

    let row = project.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO project (\"id\", \"name\", \"slug\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4)",
                (row.id, row.name, row.slug, row.created_at),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| {
            // The unique index is the only thing that can fail here,
            // and "constraint failed" says nothing an operator can act
            // on.
            if error.to_string().contains("UNIQUE") {
                PlatformError::Refused(format!(
                    "a project already uses the name {name:?} — pick another"
                ))
            } else {
                PlatformError::Storage(error)
            }
        })?;

    tracing::info!(project = %project.slug, "created");
    Ok(project)
}

pub async fn all(database: &SqliteDatabase) -> PlatformResult<Vec<Project>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"name\", \"slug\", \"created_at\" FROM project \
                     ORDER BY \"created_at\" ASC",
                )?
                .query_map([], |row| {
                    Ok(Project {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        slug: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                })?
                .collect()
        })
        .await?)
}

pub async fn find(database: &SqliteDatabase, id: &str) -> PlatformResult<Option<Project>> {
    let id = id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"id\", \"name\", \"slug\", \"created_at\" FROM project \
                     WHERE \"id\" = ?1 OR \"slug\" = ?1",
                    [id],
                    |row| {
                        Ok(Project {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            slug: row.get(2)?,
                            created_at: row.get(3)?,
                        })
                    },
                )
                .optional()
        })
        .await?)
}

/// Remove a project and every service under it.
///
/// The caller stops the containers first — this only removes rows, and
/// a container whose service row is gone is one nothing will ever
/// clean up.
pub async fn delete(database: &SqliteDatabase, id: &str) -> PlatformResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            // `ON DELETE CASCADE` takes the services, and foreign keys
            // are on — see the connection pragmas.
            connection.execute("DELETE FROM project WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> SqliteDatabase {
        crate::db::open_in_memory().await.expect("open")
    }

    #[tokio::test]
    async fn a_project_round_trips() {
        let database = database().await;
        let project = create(&database, "My API").await.expect("created");
        assert_eq!(project.slug, "my-api");

        let found = find(&database, &project.id)
            .await
            .expect("find")
            .expect("present");
        assert_eq!(found, project);

        // Findable by slug too, which is what a URL carries.
        assert_eq!(
            find(&database, "my-api")
                .await
                .expect("find")
                .expect("present")
                .id,
            project.id
        );
    }

    #[tokio::test]
    async fn two_projects_cannot_share_a_slug() {
        let database = database().await;
        create(&database, "My API").await.expect("created");

        let error = create(&database, "my api").await.expect_err("refused");
        assert!(error.to_string().contains("already uses"), "{error}");
    }

    /// A name with nothing sluggable cannot become a hostname, and
    /// finding that out at deploy time would be much later.
    #[tokio::test]
    async fn a_name_with_no_slug_is_refused() {
        let database = database().await;
        let error = create(&database, "???").await.expect_err("refused");
        assert!(error.to_string().contains("hostname"), "{error}");
    }

    #[tokio::test]
    async fn deleting_a_project_takes_its_services() {
        let database = database().await;
        let project = create(&database, "doomed").await.expect("created");
        super::super::services::create(
            &database,
            &project.id,
            "api",
            "docker.io/library/nginx:alpine",
            None,
            &[],
        )
        .await
        .expect("service");

        delete(&database, &project.id).await.expect("delete");

        assert!(super::super::services::all(&database, Some(&project.id))
            .await
            .expect("query")
            .is_empty());
    }
}
