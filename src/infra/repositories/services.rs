//! Every `SELECT`, `INSERT` and `UPDATE` a [`Service`] needs.

use std::collections::BTreeMap;

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::{DesiredState, Kind, Service};
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::domain::values::image_reference::ImageReference;
use crate::infra::clock::now_ms;

/// One service by its slug inside a project.
///
/// Slugs are unique per project, not per node — which is the point of
/// projects — so the lookup needs both halves. This is what every
/// console page has: a URL with the two of them in it.
/// The columns `decode` reads, in the order it reads them.
///
/// **One list, because three queries share one decoder.** They were
/// written out three times, and adding a column meant remembering all
/// three — miss one and it is not a compile error but
/// `InvalidColumnIndex(14)` at runtime, from whichever page happened to
/// use that query. Which is exactly what adding `cpu_millicores` did.
const COLUMNS: &str = "\"id\", \"project_id\", \"name\", \"slug\", \"image\", \"env\", \
     \"desired_state\", \"last_error\", \"address\", \"track_tag\", \"auto_deploy\", \
     \"origin_node_id\", \"memory_limit\", \"kind\", \"cpu_millicores\", \
     \"log_timestamps\", \"log_budget\", \"disk_limit\"";
pub async fn in_project(
    database: &SqliteDatabase,
    project_id: &str,
    slug: &str,
) -> PlatformResult<Option<Service>> {
    let (project_id, slug) = (project_id.to_string(), slug.to_string());
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!(
                        "SELECT {COLUMNS} FROM service \
                         WHERE \"project_id\" = ?1 AND \"slug\" = ?2"
                    ),
                    (project_id, slug),
                    decode,
                )
                .optional()
        })
        .await?)
}

pub async fn all(
    database: &SqliteDatabase,
    project_id: Option<&str>,
) -> PlatformResult<Vec<Service>> {
    let filter = project_id.map(str::to_string);
    Ok(database
        .read(move |connection| {
            let sql = format!("SELECT {COLUMNS} FROM service");
            match filter {
                Some(project) => connection
                    .prepare(&format!(
                        "{sql} WHERE \"project_id\" = ?1 ORDER BY \"slug\""
                    ))?
                    .query_map([project], decode)?
                    .collect(),
                None => connection
                    .prepare(&format!("{sql} ORDER BY \"project_id\", \"slug\""))?
                    .query_map([], decode)?
                    .collect(),
            }
        })
        .await?)
}

fn decode(row: &wabot::sqlite::rusqlite::Row<'_>) -> wabot::sqlite::rusqlite::Result<Service> {
    let env: String = row.get(5)?;
    Ok(Service {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        slug: row.get(3)?,
        image: row.get(4)?,
        // A row we cannot parse should still list, without its
        // environment, rather than take the whole page down.
        env: serde_json::from_str(&env).unwrap_or_default(),
        desired_state: DesiredState::parse(&row.get::<_, String>(6)?),
        last_error: row.get(7)?,
        address: row.get(8)?,
        track_tag: row.get(9)?,
        auto_deploy: row.get::<_, i64>(10)? != 0,
        origin_node_id: row.get(11)?,
        memory_limit: row.get::<_, Option<i64>>(12)?.map(|bytes| bytes as u64),
        kind: Kind::parse(&row.get::<_, String>(13)?),
        cpu_millicores: row.get::<_, Option<i64>>(14)?.map(|milli| milli as u32),
        log_timestamps: row.get::<_, i64>(15)? != 0,
        log_budget: row.get::<_, Option<i64>>(16)?.map(|bytes| bytes as u64),
        disk_limit: row.get::<_, Option<i64>>(17)?.map(|bytes| bytes as u64),
    })
}

#[allow(dead_code)]
pub async fn set_image(
    database: &SqliteDatabase,
    service_id: &str,
    image: &str,
) -> PlatformResult<()> {
    let image = ImageReference::parse(image)?;
    let (service_id, image) = (service_id.to_string(), image.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"image\" = ?2 WHERE \"id\" = ?1",
                (service_id, image),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn set_origin(
    database: &SqliteDatabase,
    service_id: &str,
    node_id: &str,
) -> PlatformResult<()> {
    let (service_id, node_id) = (service_id.to_string(), node_id.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"origin_node_id\" = ?2 WHERE \"id\" = ?1",
                (service_id, node_id),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn set_desired_state(
    database: &SqliteDatabase,
    service_id: &str,
    state: DesiredState,
) -> PlatformResult<()> {
    let id = service_id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"desired_state\" = ?2, \"updated_at\" = ?3 WHERE \"id\" = ?1",
                (id, state.as_str(), now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Record why a deployment failed, or clear it because one worked.
// Exercised by this module's tests, but nothing in the running binary
// calls it yet: the deploy path is what will, and it is the next
// milestone. `allow` rather than `expect` because the two builds
// disagree — under `--all-targets` the tests fulfil it and the
// expectation itself becomes the warning.
pub async fn set_last_error(
    database: &SqliteDatabase,
    service_id: &str,
    error: Option<&str>,
) -> PlatformResult<()> {
    let (id, error) = (service_id.to_string(), error.map(str::to_string));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"last_error\" = ?2, \"updated_at\" = ?3 WHERE \"id\" = ?1",
                (id, error, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn set_tracking(
    database: &SqliteDatabase,
    service_id: &str,
    track_tag: Option<&str>,
    auto_deploy: bool,
) -> PlatformResult<()> {
    // A partial glob is refused rather than stored as a literal tag.
    // `*` on its own means every tag; `v*` would otherwise be saved as the
    // name of a tag nobody will ever push, and a setting that silently
    // matches nothing is the quietest kind of wrong.
    if let Some(tag) = track_tag.map(str::trim).filter(|tag| !tag.is_empty()) {
        if tag.contains('*') && tag != crate::domain::entities::service::EVERY_TAG {
            return Err(PlatformError::Refused(format!(
                "{tag:?} is not a tag. Use `*` on its own to watch every tag, or write one \
                 tag exactly — patterns are not matched."
            )));
        }
    }

    let (id, tag) = (service_id.to_string(), track_tag.map(str::to_string));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"track_tag\" = ?2, \"auto_deploy\" = ?3, \
                 \"updated_at\" = ?4 WHERE \"id\" = ?1",
                (id, tag, i64::from(auto_deploy), now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn find(database: &SqliteDatabase, id: &str) -> PlatformResult<Option<Service>> {
    let id = id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM service WHERE \"id\" = ?1"),
                    [id],
                    decode,
                )
                .optional()
        })
        .await?)
}

pub async fn set_kind(
    database: &SqliteDatabase,
    service_id: &str,
    kind: Kind,
) -> PlatformResult<()> {
    let id = service_id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"kind\" = ?2, \"updated_at\" = ?3 WHERE \"id\" = ?1",
                (id, kind.as_str(), now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn set_memory_limit(
    database: &SqliteDatabase,
    service_id: &str,
    bytes: Option<u64>,
) -> PlatformResult<()> {
    // **A floor, not a list.** The ladder used to be the rule here, and
    // it was the wrong shape for a plain container: an operator who knows
    // their service needs 300 MB had to pick 256 or 512. What the machine
    // actually requires is a minimum — below it nothing starts.
    //
    // A *database's* ceiling is still a rung, and that check stays in
    // `databases`, which is where the reason lives: the size picks
    // `shared_buffers` and five other numbers, so it is a configuration
    // and not only a limit. Enforced there rather than here because this
    // function is handed an id and would have to read the row back to
    // learn the kind.
    if let Some(bytes) = bytes {
        if bytes < crate::domain::policies::presets::MIN_MEMORY {
            return Err(PlatformError::Refused(format!(
                "{} is under {}, which is less than a container needs to start",
                crate::domain::policies::presets::label(bytes),
                crate::domain::policies::presets::label(
                    crate::domain::policies::presets::MIN_MEMORY
                )
            )));
        }
    }
    let (id, bytes) = (service_id.to_string(), bytes.map(|bytes| bytes as i64));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"memory_limit\" = ?2, \"updated_at\" = ?3 WHERE \"id\" = ?1",
                (id, bytes, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn set_cpu_limit(
    database: &SqliteDatabase,
    service_id: &str,
    millicores: Option<u32>,
) -> PlatformResult<()> {
    if let Some(millicores) = millicores {
        if millicores < crate::domain::policies::presets::MIN_MILLICORES {
            return Err(PlatformError::Refused(format!(
                "{} is under {}, which is less than a container needs to start",
                crate::domain::policies::presets::cpu_label(millicores),
                crate::domain::policies::presets::cpu_label(
                    crate::domain::policies::presets::MIN_MILLICORES
                )
            )));
        }
    }
    let (id, millicores) = (service_id.to_string(), millicores.map(i64::from));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"cpu_millicores\" = ?2, \"updated_at\" = ?3 \
                 WHERE \"id\" = ?1",
                (id, millicores, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn set_disk_limit(
    database: &SqliteDatabase,
    service_id: &str,
    bytes: Option<u64>,
) -> PlatformResult<()> {
    if let Some(bytes) = bytes {
        if bytes < crate::domain::policies::presets::MIN_MEMORY {
            return Err(PlatformError::Refused(format!(
                "a disk ceiling under {} is one nothing could start in",
                crate::domain::policies::presets::label(
                    crate::domain::policies::presets::MIN_MEMORY
                )
            )));
        }
    }
    let (id, bytes) = (service_id.to_string(), bytes.map(|bytes| bytes as i64));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"disk_limit\" = ?2, \"updated_at\" = ?3 \
                 WHERE \"id\" = ?1",
                (id, bytes, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn set_log_budget(
    database: &SqliteDatabase,
    service_id: &str,
    bytes: Option<u64>,
) -> PlatformResult<()> {
    let (id, bytes) = (service_id.to_string(), bytes.map(|bytes| bytes as i64));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"log_budget\" = ?2, \"updated_at\" = ?3 WHERE \"id\" = ?1",
                (id, bytes, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn set_log_timestamps(
    database: &SqliteDatabase,
    service_id: &str,
    on: bool,
) -> PlatformResult<()> {
    let id = service_id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"log_timestamps\" = ?2, \"updated_at\" = ?3 \
                 WHERE \"id\" = ?1",
                (id, i64::from(on), now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn set_env(
    database: &SqliteDatabase,
    service_id: &str,
    env: &BTreeMap<String, String>,
) -> PlatformResult<()> {
    let id = service_id.to_string();
    let payload = serde_json::to_string(env).unwrap_or_else(|_| "{}".into());
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"env\" = ?2, \"updated_at\" = ?3 WHERE \"id\" = ?1",
                (id, payload, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn delete(database: &SqliteDatabase, service_id: &str) -> PlatformResult<()> {
    let id = service_id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM service WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn create(
    database: &SqliteDatabase,
    project_id: &str,
    name: &str,
    image: &str,
    env: &[(String, String)],
) -> PlatformResult<Service> {
    let service = Service::new(project_id, name, image, env)?;

    let row = service.clone();
    let env_json = serde_json::to_string(&row.env).unwrap_or_else(|_| "{}".into());
    let (revision_of, revision_env) = (row.id.clone(), row.env.clone());
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
            crate::infra::repositories::config_history::insert(
                connection,
                &revision_of,
                &revision_env,
                None,
                "create",
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
    crate::infra::repositories::replicas::ensure_here(database, &service.id, 1).await?;

    Ok(service)
}

#[cfg(test)]
mod tests {

    /// The picker and the door have to name the same capability.
    ///
    /// **They did not, and the copy vanished between them.** The
    /// placement form offered nodes on `host`; a managed database's copy
    /// travels as `errand::Kind::Database`, which
    /// [`errand::consent_for`] answers `store` for. So a node with `host`
    /// and not `store` was offered, chosen, written as a replica row —
    /// and then skipped in silence at dispatch, leaving a copy the
    /// console showed as placed that was never sent anywhere.
    ///
    /// Asked against `consent_for` rather than against a literal, because
    /// a test that restated the mapping would agree with a wrong mapping
    /// just as happily.
    #[test]
    fn what_is_offered_is_what_the_far_side_will_consent_to() {
        use crate::infra::network::errand;

        // Which errand carries a copy of each kind, which is the pairing
        // `holders` and `crate::infra::deploy::standbys::dispatch` make between them.
        for (kind, carried_by) in [
            (Kind::Container, errand::Kind::Host),
            (Kind::Postgres, errand::Kind::Database),
            (Kind::Unknown, errand::Kind::Database),
        ] {
            assert_eq!(
                Some(kind.placement_capability()),
                errand::consent_for(carried_by, &serde_json::Value::Null),
                "a {} copy is offered on one capability and consented to as another",
                kind.as_str()
            );
        }
    }

    /// containerd's own rule, copied from the error it answers with.
    /// A container id it refuses is a deployment that fails after the
    /// network is already built.
    #[test]
    fn a_container_id_is_one_containerd_accepts() {
        let service = Service {
            log_budget: None,
            id: "svc-1".into(),
            project_id: "prj-1".into(),
            name: "nginx".into(),
            slug: "nginx".into(),
            image: "docker.io/library/nginx:alpine".into(),
            env: Default::default(),
            desired_state: DesiredState::Running,
            last_error: None,
            address: None,
            track_tag: None,
            origin_node_id: None,
            auto_deploy: true,
            memory_limit: None,
            cpu_millicores: None,
            disk_limit: None,
            log_timestamps: false,
            kind: Kind::Container,
        };

        let id = service.container_id("first-project");
        assert_eq!(id, "first-project.nginx");

        assert!(containerd_accepts(&id), "containerd would refuse {id}");
        // The shape that broke it: a doubled separator.
        assert!(!containerd_accepts("first-project--nginx"));
        assert!(!containerd_accepts("-leading"));
        assert!(!containerd_accepts("trailing-"));
    }

    /// containerd's rule — `^[A-Za-z0-9]+(?:[._-][A-Za-z0-9]+)*$` —
    /// spelled out rather than pulled in as a regex engine: runs of
    /// alphanumerics, joined by single separators, starting and ending
    /// with a run.
    fn containerd_accepts(id: &str) -> bool {
        !id.is_empty()
            && id
                .split(['.', '_', '-'])
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric()))
    }

    use super::*;
}

#[cfg(test)]
mod creating {
    use super::*;
    use crate::domain::entities::service::DesiredState;
    use wabot::sqlite::SqliteDatabase;

    async fn project() -> (SqliteDatabase, String) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        (database, project.id)
    }

    #[tokio::test]
    async fn a_service_round_trips_with_its_environment() {
        let (database, project_id) = project().await;
        let service = crate::infra::repositories::services::create(
            &database,
            &project_id,
            "My API",
            "docker.io/library/nginx:alpine",
            &[("LOG_LEVEL".into(), "debug".into())],
        )
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
        let second = crate::infra::repositories::projects::create(&database, "other")
            .await
            .expect("project")
            .id;

        for project_id in [&first, &second] {
            crate::infra::repositories::services::create(
                &database,
                project_id,
                "api",
                "docker.io/library/nginx:alpine",
                &[],
            )
            .await
            .expect("created");
        }

        let error = crate::infra::repositories::services::create(
            &database,
            &first,
            "api",
            "docker.io/library/nginx:alpine",
            &[],
        )
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
        let error = crate::infra::repositories::services::create(
            &database,
            &project_id,
            "api",
            "https://docker.io/library/nginx",
            &[],
        )
        .await
        .expect_err("refused");
        assert!(error.to_string().contains("not a URL"), "{error}");
    }
    /// The distinction the console rests on: what was asked for is not
    /// what is happening.
    #[tokio::test]
    async fn desired_state_is_recorded_separately() {
        let (database, project_id) = project().await;
        let service = crate::infra::repositories::services::create(
            &database,
            &project_id,
            "api",
            "docker.io/library/nginx:alpine",
            &[],
        )
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
        let service = crate::infra::repositories::services::create(
            &database,
            &project_id,
            "api",
            "docker.io/library/nginx:alpine",
            &[],
        )
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
        let service = crate::infra::repositories::services::create(
            &database,
            &project_id,
            "My API",
            "docker.io/library/nginx:alpine",
            &[],
        )
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
        let service = crate::infra::repositories::services::create(
            &database,
            &project_id,
            "api",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("created");
        assert_eq!(service.memory_limit, None);

        let rung = crate::domain::policies::presets::LADDER[1];
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
        let service = crate::infra::repositories::services::create(
            &database,
            &project_id,
            "api",
            "docker.io/library/nginx:alpine",
            &[],
        )
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
