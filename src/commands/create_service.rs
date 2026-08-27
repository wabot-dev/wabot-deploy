use wabot::sqlite::SqliteDatabase;

use crate::entities::service::Service;
use crate::platform::{now_ms, replicas, PlatformError, PlatformResult};

pub struct Asked<'a> {
    pub project: &'a str,
    pub name: &'a str,
    pub image: &'a str,
}

pub struct CreateService<'a> {
    pub project_id: &'a str,
    pub name: &'a str,
    pub image: &'a str,
    pub env: &'a [(String, String)],
}

impl CreateService<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> PlatformResult<Service> {
        let service = Service::new(self.project_id, self.name, self.image, self.env)?;

        let row = service.clone();
        let env_json = serde_json::to_string(&row.env).unwrap_or_else(|_| "{}".into());
        database
            .write(move |connection| {
                connection.execute(
                    "INSERT INTO service \
                       (\"id\", \"project_id\", \"name\", \"slug\", \"image\", \
                        \"env\", \"desired_state\", \"created_at\", \"updated_at\") \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    (
                        row.id,
                        row.project_id,
                        row.name,
                        row.slug,
                        row.image,
                        env_json,
                        row.desired_state.as_str(),
                        now_ms(),
                    ),
                )?;
                Ok(())
            })
            .await
            .map_err(|error| {
                if error.to_string().contains("UNIQUE") {
                    PlatformError::Refused(format!(
                        "this project already has a service called {:?}",
                        service.name
                    ))
                } else if error.to_string().contains("FOREIGN KEY") {
                    PlatformError::Refused("no such project".into())
                } else {
                    PlatformError::Storage(error)
                }
            })?;

        tracing::info!(service = %service.slug, image = %service.image, "created");
        // A service is at least one replica, and creating it is where that
        // becomes true. Without this a new service is a description of
        // something with nowhere to run — and reconciliation, which asks
        // about replicas, would never start it.
        //
        // Services that predate replicas were given theirs by migration
        // `0020`; this is the same row, for the ones made since.
        replicas::ensure_here(database, &service.id, 1).await?;

        Ok(service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::service::DesiredState;
    use crate::repositories::services::{
        in_project, set_desired_state, set_last_error, set_memory_limit,
    };
    use wabot::sqlite::SqliteDatabase;

    async fn project() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = crate::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        (database, project.id)
    }

    #[tokio::test]
    async fn a_service_round_trips_with_its_environment() {
        let (database, project_id) = project().await;
        let service = CreateService {
            project_id: &project_id,
            name: "My API",
            image: "docker.io/library/nginx:alpine",
            env: &[("LOG_LEVEL".into(), "debug".into())],
        }
        .run(&database)
        .await
        .expect("created");

        assert_eq!(service.slug, "my-api");
        let found = in_project(&database, &project_id, &service.slug)
            .await
            .expect("find")
            .expect("present");
        assert_eq!(
            found.env.get("LOG_LEVEL").map(String::as_str),
            Some("debug")
        );
        assert_eq!(found.desired_state, DesiredState::Running);
    }
    /// Two projects may each have an `api`. Being able to is the point
    /// of projects.
    #[tokio::test]
    async fn the_same_name_is_free_in_another_project() {
        let (database, first) = project().await;
        let second = crate::repositories::projects::create(&database, "other")
            .await
            .expect("project")
            .id;

        for project_id in [&first, &second] {
            CreateService {
                project_id,
                name: "api",
                image: "nginx:alpine",
                env: &[],
            }
            .run(&database)
            .await
            .expect("created");
        }

        let error = CreateService {
            project_id: &first,
            name: "api",
            image: "nginx:alpine",
            env: &[],
        }
        .run(&database)
        .await
        .expect_err("refused");
        assert!(
            error.to_string().contains("already has a service"),
            "{error}"
        );
    }
    /// The mistake somebody makes once, and the error should say so
    /// rather than failing later at the registry.
    #[tokio::test]
    async fn a_url_is_not_an_image_reference() {
        let (database, project_id) = project().await;
        let error = CreateService {
            project_id: &project_id,
            name: "api",
            image: "https://docker.io/library/nginx",
            env: &[],
        }
        .run(&database)
        .await
        .expect_err("refused");
        assert!(error.to_string().contains("not a URL"), "{error}");
    }
    /// The distinction the console rests on: what was asked for is not
    /// what is happening.
    #[tokio::test]
    async fn desired_state_is_recorded_separately() {
        let (database, project_id) = project().await;
        let service = CreateService {
            project_id: &project_id,
            name: "api",
            image: "nginx:alpine",
            env: &[],
        }
        .run(&database)
        .await
        .expect("created");
        assert_eq!(service.desired_state, DesiredState::Running);

        set_desired_state(&database, &service.id, DesiredState::Stopped)
            .await
            .expect("stop");
        assert_eq!(
            in_project(&database, &project_id, &service.slug)
                .await
                .expect("find")
                .expect("present")
                .desired_state,
            DesiredState::Stopped
        );
    }
    #[tokio::test]
    async fn a_failure_is_recorded_and_cleared() {
        let (database, project_id) = project().await;
        let service = CreateService {
            project_id: &project_id,
            name: "api",
            image: "nginx:alpine",
            env: &[],
        }
        .run(&database)
        .await
        .expect("created");

        set_last_error(&database, &service.id, Some("no such image"))
            .await
            .expect("record");
        assert_eq!(
            in_project(&database, &project_id, &service.slug)
                .await
                .unwrap()
                .unwrap()
                .last_error,
            Some("no such image".to_string())
        );

        set_last_error(&database, &service.id, None)
            .await
            .expect("clear");
        assert_eq!(
            in_project(&database, &project_id, &service.slug)
                .await
                .unwrap()
                .unwrap()
                .last_error,
            None
        );
    }
    /// The container id has to be reconstructible from the row, because
    /// reconciliation starts from rows and asks containerd what it has.
    #[tokio::test]
    async fn the_container_id_names_its_project_and_service() {
        let (database, project_id) = project().await;
        let service = CreateService {
            project_id: &project_id,
            name: "My API",
            image: "nginx:alpine",
            env: &[],
        }
        .run(&database)
        .await
        .expect("created");
        assert_eq!(service.container_id("demo"), "demo.my-api");
    }
    /// No ceiling is what every service had before there were presets,
    /// and a size off the ladder is refused rather than stored — a
    /// number nothing offers is one nothing can show back.
    #[tokio::test]
    async fn a_memory_ceiling_is_one_of_the_sizes_on_offer() {
        let (database, project_id) = project().await;
        let service = CreateService {
            project_id: &project_id,
            name: "api",
            image: "nginx:alpine",
            env: &[],
        }
        .run(&database)
        .await
        .expect("created");
        assert_eq!(service.memory_limit, None);

        let rung = crate::policies::presets::LADDER[1];
        set_memory_limit(&database, &service.id, Some(rung))
            .await
            .expect("set");
        assert_eq!(
            in_project(&database, &project_id, &service.slug)
                .await
                .unwrap()
                .unwrap()
                .memory_limit,
            Some(rung)
        );

        assert!(
            set_memory_limit(&database, &service.id, Some(100))
                .await
                .is_err(),
            "a size nothing offers was stored"
        );

        set_memory_limit(&database, &service.id, None)
            .await
            .expect("clear");
        assert_eq!(
            in_project(&database, &project_id, &service.slug)
                .await
                .unwrap()
                .unwrap()
                .memory_limit,
            None
        );
    }
    /// A row with unreadable JSON should list without its environment
    /// rather than take the page down.
    #[tokio::test]
    async fn a_corrupt_environment_does_not_break_the_listing() {
        let (database, project_id) = project().await;
        let service = CreateService {
            project_id: &project_id,
            name: "api",
            image: "nginx:alpine",
            env: &[],
        }
        .run(&database)
        .await
        .expect("created");

        let id = service.id.clone();
        database
            .write(move |connection| {
                connection.execute(
                    "UPDATE service SET \"env\" = 'not json' WHERE \"id\" = ?1",
                    [id],
                )?;
                Ok(())
            })
            .await
            .expect("corrupt it");

        let found = in_project(&database, &project_id, &service.slug)
            .await
            .expect("find")
            .expect("present");
        assert!(found.env.is_empty());
    }
}
