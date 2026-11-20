//! What a project runs on other nodes, as its owner last said.

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use crate::domain::errors::PlatformResult;
use crate::infra::clock::now_ms;
use crate::infra::network::errand::Directory;

/// Keep `directory` for the project, or forget it when it lists nothing.
pub async fn keep(
    database: &SqliteDatabase,
    project_id: &str,
    directory: &Directory,
) -> PlatformResult<()> {
    let project = project_id.to_string();
    let body = match directory.services.is_empty() {
        true => None,
        false => Some(serde_json::to_string(directory).expect("a directory is plain data")),
    };
    database
        .write(move |connection| {
            match body {
                Some(body) => connection.execute(
                    "INSERT INTO project_directory (\"project_id\", \"body\", \"updated_at\") \
                     VALUES (?1, ?2, ?3) \
                     ON CONFLICT (\"project_id\") DO UPDATE SET \
                       \"body\" = excluded.\"body\", \"updated_at\" = excluded.\"updated_at\"",
                    (project, body, now_ms()),
                )?,
                None => connection.execute(
                    "DELETE FROM project_directory WHERE \"project_id\" = ?1",
                    [project],
                )?,
            };
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn of_project(
    database: &SqliteDatabase,
    project_id: &str,
) -> PlatformResult<Option<Directory>> {
    let project = project_id.to_string();
    let body: Option<String> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"body\" FROM project_directory WHERE \"project_id\" = ?1",
                    [project],
                    |row| row.get(0),
                )
                .optional()
        })
        .await?;
    Ok(body.and_then(|body| serde_json::from_str(&body).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::network::errand::{Copy, Listed};

    #[tokio::test]
    async fn a_directory_listing_nothing_is_forgotten_rather_than_kept_empty() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");
        let listed = Directory {
            project: "shop".into(),
            members: vec!["10.42.0.1".into(), "10.42.0.2".into()],
            services: vec![Listed {
                service: "orders".into(),
                qualified: None,
                ports: vec![5432],
                primary_slot: Some(1),
                copies: vec![Copy {
                    node: "10.42.0.1".into(),
                    slot: 1,
                }],
            }],
        };

        keep(&database, &project.id, &listed).await.expect("kept");
        assert_eq!(
            of_project(&database, &project.id).await.expect("read"),
            Some(listed)
        );

        keep(
            &database,
            &project.id,
            &Directory {
                project: "shop".into(),
                ..Directory::default()
            },
        )
        .await
        .expect("withdrawn");
        assert_eq!(
            of_project(&database, &project.id).await.expect("read"),
            None
        );
    }
}
