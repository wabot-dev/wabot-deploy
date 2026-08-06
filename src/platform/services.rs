//! Services: an image, a port, and what the operator wants it doing.
//!
//! ## Desired state is not observed state
//!
//! `desired_state` is what was asked for. What containerd reports is
//! what *is*. A service can be desired-running and crashed, and
//! collapsing the two loses the only field that says which — so the
//! console can never tell "stopped on purpose" from "fell over".

use std::collections::BTreeMap;

use serde::Serialize;
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::{now_ms, slugify, PlatformError, PlatformResult};

/// Where the node allocates host ports from.
///
/// Above the registered range and below the ephemeral one Linux hands
/// out for outgoing connections, so an allocation cannot collide with
/// a socket the kernel opened while nobody was looking.
const PORT_RANGE: std::ops::Range<u16> = 20000..29000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredState {
    Running,
    Stopped,
}

impl DesiredState {
    pub fn as_str(self) -> &'static str {
        match self {
            DesiredState::Running => "running",
            DesiredState::Stopped => "stopped",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "stopped" => DesiredState::Stopped,
            // Anything unrecognised reads as running: a row we cannot
            // interpret should keep serving rather than quietly stop.
            _ => DesiredState::Running,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Service {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub slug: String,
    pub image: String,
    pub container_port: Option<u16>,
    pub host_port: Option<u16>,
    pub env: BTreeMap<String, String>,
    pub desired_state: DesiredState,
    pub last_error: Option<String>,
    /// The container's address on its project's bridge, while it is
    /// running. `None` means nothing to proxy to.
    pub address: Option<String>,
}

impl Service {
    /// The containerd container id, and the label every container of
    /// this node carries.
    ///
    /// Derived rather than stored: it has to be reconstructible from
    /// the row alone, because reconciliation on boot starts from rows
    /// and asks containerd what it has.
    ///
    /// A dot joins them, for two reasons that agree. containerd
    /// validates ids against `^[A-Za-z0-9]+(?:[._-][A-Za-z0-9]+)*$`,
    /// which allows single separators only — `project--service` is
    /// refused outright, as this found out on a real node. And a slug
    /// is `[a-z0-9-]`, so a dot cannot occur inside either half: the
    /// id parses back apart unambiguously, which `-` would not.
    pub fn container_id(&self, project_slug: &str) -> String {
        format!("{project_slug}.{}", self.slug)
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create(
    database: &SqliteDatabase,
    project_id: &str,
    name: &str,
    image: &str,
    container_port: Option<u16>,
    env: &[(String, String)],
) -> PlatformResult<Service> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(PlatformError::Refused(
            "a service name is between 1 and 60 characters".into(),
        ));
    }
    let slug = slugify(name);
    if slug.is_empty() {
        return Err(PlatformError::Refused(
            "that name has nothing a hostname can be built from".into(),
        ));
    }
    validate_image(image)?;

    let service = Service {
        id: format!("svc-{}", wabot::prelude::password::generate(12)),
        project_id: project_id.to_string(),
        name: name.to_string(),
        slug,
        image: image.trim().to_string(),
        container_port,
        // Allocated at deploy, not at create: a service that never
        // runs should not be holding a port.
        host_port: None,
        env: env.iter().cloned().collect(),
        desired_state: DesiredState::Running,
        last_error: None,
        address: None,
    };

    let row = service.clone();
    let env_json = serde_json::to_string(&row.env).unwrap_or_else(|_| "{}".into());
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO service \
                   (\"id\", \"project_id\", \"name\", \"slug\", \"image\", \"container_port\", \
                    \"env\", \"desired_state\", \"created_at\", \"updated_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                (
                    row.id,
                    row.project_id,
                    row.name,
                    row.slug,
                    row.image,
                    row.container_port,
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
                    "this project already has a service called {name:?}"
                ))
            } else if error.to_string().contains("FOREIGN KEY") {
                PlatformError::Refused("no such project".into())
            } else {
                PlatformError::Storage(error)
            }
        })?;

    tracing::info!(service = %service.slug, image = %service.image, "created");
    Ok(service)
}

/// A reference has to name something containerd can resolve.
///
/// Not a full parser — the registry will have the last word — just the
/// mistakes worth catching before a deployment goes looking: an empty
/// string, a scheme somebody pasted, whitespace in the middle.
fn validate_image(image: &str) -> PlatformResult<()> {
    let image = image.trim();
    if image.is_empty() {
        return Err(PlatformError::Refused(
            "an image reference is required".into(),
        ));
    }
    if image.contains("://") {
        return Err(PlatformError::Refused(format!(
            "an image reference is not a URL: {image:?} — try docker.io/library/nginx:alpine"
        )));
    }
    if image.split_whitespace().count() != 1 {
        return Err(PlatformError::Refused(
            "an image reference has no spaces in it".into(),
        ));
    }
    Ok(())
}

pub async fn all(
    database: &SqliteDatabase,
    project_id: Option<&str>,
) -> PlatformResult<Vec<Service>> {
    let filter = project_id.map(str::to_string);
    Ok(database
        .read(move |connection| {
            let sql = "SELECT \"id\", \"project_id\", \"name\", \"slug\", \"image\", \
                       \"container_port\", \"host_port\", \"env\", \"desired_state\", \
                       \"last_error\", \"address\" FROM service";
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

pub async fn find(database: &SqliteDatabase, id: &str) -> PlatformResult<Option<Service>> {
    let id = id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"id\", \"project_id\", \"name\", \"slug\", \"image\", \
                     \"container_port\", \"host_port\", \"env\", \"desired_state\", \
                     \"last_error\", \"address\" FROM service WHERE \"id\" = ?1",
                    [id],
                    decode,
                )
                .optional()
        })
        .await?)
}

fn decode(row: &wabot::sqlite::rusqlite::Row<'_>) -> wabot::sqlite::rusqlite::Result<Service> {
    let env: String = row.get(7)?;
    Ok(Service {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        slug: row.get(3)?,
        image: row.get(4)?,
        container_port: row.get::<_, Option<i64>>(5)?.map(|port| port as u16),
        host_port: row.get::<_, Option<i64>>(6)?.map(|port| port as u16),
        // A row we cannot parse should still list, without its
        // environment, rather than take the whole page down.
        env: serde_json::from_str(&env).unwrap_or_default(),
        desired_state: DesiredState::parse(&row.get::<_, String>(8)?),
        last_error: row.get(9)?,
        address: row.get(10)?,
    })
}

/// Give this service a host port, if it has none.
///
/// Containers share the host's network, so a port is a node-wide
/// resource and two services cannot hold the same one. The unique
/// index is what makes this safe rather than the check: two concurrent
/// allocations both see a free port and one insert loses, which is the
/// correct outcome and the reason to retry.
// Exercised by this module's tests, but nothing in the running binary
// calls it yet: the deploy path is what will, and it is the next
// milestone. `allow` rather than `expect` because the two builds
// disagree — under `--all-targets` the tests fulfil it and the
// expectation itself becomes the warning.
#[allow(dead_code)]
pub async fn allocate_port(database: &SqliteDatabase, service_id: &str) -> PlatformResult<u16> {
    if let Some(service) = find(database, service_id).await? {
        if let Some(port) = service.host_port {
            return Ok(port);
        }
    }

    let taken: Vec<u16> = database
        .read(|connection| {
            connection
                .prepare("SELECT \"host_port\" FROM service WHERE \"host_port\" IS NOT NULL")?
                .query_map([], |row| row.get::<_, i64>(0))?
                .map(|port| port.map(|port| port as u16))
                .collect()
        })
        .await?;

    let port = PORT_RANGE
        .clone()
        .find(|port| !taken.contains(port))
        .ok_or_else(|| {
            PlatformError::Refused(format!(
                "every port in {}..{} is taken — that is {} services on one node",
                PORT_RANGE.start,
                PORT_RANGE.end,
                taken.len()
            ))
        })?;

    let id = service_id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"host_port\" = ?2, \"updated_at\" = ?3 WHERE \"id\" = ?1",
                (id, port as i64, now_ms()),
            )?;
            Ok(())
        })
        .await?;

    Ok(port)
}

// Exercised by this module's tests, but nothing in the running binary
// calls it yet: the deploy path is what will, and it is the next
// milestone. `allow` rather than `expect` because the two builds
// disagree — under `--all-targets` the tests fulfil it and the
// expectation itself becomes the warning.
#[allow(dead_code)]
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
#[allow(dead_code)]
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

/// Where the proxy reaches this service, or `None` because it is not
/// running.
pub async fn set_address(
    database: &SqliteDatabase,
    service_id: &str,
    address: Option<&str>,
) -> PlatformResult<()> {
    let (id, address) = (service_id.to_string(), address.map(str::to_string));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE service SET \"address\" = ?2, \"updated_at\" = ?3 WHERE \"id\" = ?1",
                (id, address, now_ms()),
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

#[cfg(test)]
mod tests {
    /// containerd's own rule, copied from the error it answers with.
    /// A container id it refuses is a deployment that fails after the
    /// network is already built.
    #[test]
    fn a_container_id_is_one_containerd_accepts() {
        let service = Service {
            id: "svc-1".into(),
            project_id: "prj-1".into(),
            name: "nginx".into(),
            slug: "nginx".into(),
            image: "docker.io/library/nginx:alpine".into(),
            container_port: Some(80),
            host_port: None,
            env: Default::default(),
            desired_state: DesiredState::Running,
            last_error: None,
            address: None,
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

    async fn project() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = super::super::projects::create(&database, "demo")
            .await
            .expect("project");
        (database, project.id)
    }

    #[tokio::test]
    async fn a_service_round_trips_with_its_environment() {
        let (database, project_id) = project().await;
        let service = create(
            &database,
            &project_id,
            "My API",
            "docker.io/library/nginx:alpine",
            Some(80),
            &[("LOG_LEVEL".into(), "debug".into())],
        )
        .await
        .expect("created");

        assert_eq!(service.slug, "my-api");
        let found = find(&database, &service.id)
            .await
            .expect("find")
            .expect("present");
        assert_eq!(
            found.env.get("LOG_LEVEL").map(String::as_str),
            Some("debug")
        );
        assert_eq!(found.container_port, Some(80));
        assert_eq!(found.desired_state, DesiredState::Running);
    }

    /// Two projects may each have an `api`. Being able to is the point
    /// of projects.
    #[tokio::test]
    async fn the_same_name_is_free_in_another_project() {
        let (database, first) = project().await;
        let second = super::super::projects::create(&database, "other")
            .await
            .expect("project")
            .id;

        for project_id in [&first, &second] {
            create(&database, project_id, "api", "nginx:alpine", None, &[])
                .await
                .expect("created");
        }

        let error = create(&database, &first, "api", "nginx:alpine", None, &[])
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
        let error = create(
            &database,
            &project_id,
            "api",
            "https://docker.io/library/nginx",
            None,
            &[],
        )
        .await
        .expect_err("refused");
        assert!(error.to_string().contains("not a URL"), "{error}");
    }

    /// Containers share the host's network, so a port is node-wide and
    /// two services holding one would be one service that never starts.
    #[tokio::test]
    async fn every_service_gets_a_different_port() {
        let (database, project_id) = project().await;
        let mut ports = Vec::new();

        for name in ["a", "b", "c"] {
            let service = create(&database, &project_id, name, "nginx:alpine", None, &[])
                .await
                .expect("created");
            ports.push(allocate_port(&database, &service.id).await.expect("port"));
        }

        let unique: std::collections::HashSet<u16> = ports.iter().copied().collect();
        assert_eq!(unique.len(), 3, "three services, three ports: {ports:?}");
        assert!(ports.iter().all(|port| PORT_RANGE.contains(port)));
    }

    /// Allocation is idempotent: a redeploy must not move a service to
    /// a new port and leave the route pointing at the old one.
    #[tokio::test]
    async fn a_service_keeps_the_port_it_was_given() {
        let (database, project_id) = project().await;
        let service = create(&database, &project_id, "api", "nginx:alpine", None, &[])
            .await
            .expect("created");

        let first = allocate_port(&database, &service.id).await.expect("port");
        let second = allocate_port(&database, &service.id).await.expect("port");
        assert_eq!(first, second);
    }

    /// The distinction the console rests on: what was asked for is not
    /// what is happening.
    #[tokio::test]
    async fn desired_state_is_recorded_separately() {
        let (database, project_id) = project().await;
        let service = create(&database, &project_id, "api", "nginx:alpine", None, &[])
            .await
            .expect("created");
        assert_eq!(service.desired_state, DesiredState::Running);

        set_desired_state(&database, &service.id, DesiredState::Stopped)
            .await
            .expect("stop");
        assert_eq!(
            find(&database, &service.id)
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
        let service = create(&database, &project_id, "api", "nginx:alpine", None, &[])
            .await
            .expect("created");

        set_last_error(&database, &service.id, Some("no such image"))
            .await
            .expect("record");
        assert_eq!(
            find(&database, &service.id)
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
            find(&database, &service.id)
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
        let service = create(&database, &project_id, "My API", "nginx:alpine", None, &[])
            .await
            .expect("created");
        assert_eq!(service.container_id("demo"), "demo.my-api");
    }

    /// A row with unreadable JSON should list without its environment
    /// rather than take the page down.
    #[tokio::test]
    async fn a_corrupt_environment_does_not_break_the_listing() {
        let (database, project_id) = project().await;
        let service = create(&database, &project_id, "api", "nginx:alpine", None, &[])
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

        let found = find(&database, &service.id)
            .await
            .expect("find")
            .expect("present");
        assert!(found.env.is_empty());
    }
}
