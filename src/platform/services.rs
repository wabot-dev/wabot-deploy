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

/// What kind of thing this service is.
///
/// Not a hint. The deploy path reads it to decide what a container
/// needs — a volume, a ceiling, tuning arguments, a role — and the
/// console reads it to decide which page to show, because a managed
/// database has no image field and no environment editor: the node
/// writes both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An image somebody chose, run as it is. Everything, until there
    /// were databases.
    Container,
    Postgres,
    /// A kind from a newer node. Held and refused rather than parsed
    /// into a panic — the same shape `errand::Kind` uses, and for the
    /// same reason: a row outlives the version that wrote it.
    Unknown,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Container => "container",
            Kind::Postgres => "postgres",
            Kind::Unknown => "unknown",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "container" => Kind::Container,
            "postgres" => Kind::Postgres,
            _ => Kind::Unknown,
        }
    }

    /// Whether the node writes this service's image, environment and
    /// arguments rather than the operator.
    ///
    /// The question every console page asks, and the one that must not
    /// be spelled `kind == Postgres` in a dozen places: a second engine
    /// would make every one of them wrong.
    pub fn is_managed(self) -> bool {
        !matches!(self, Kind::Container)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Service {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub slug: String,
    pub image: String,
    pub env: BTreeMap<String, String>,
    pub desired_state: DesiredState,
    pub last_error: Option<String>,
    /// Vestigial. An address belongs to a **replica** now — a service
    /// is *n* running copies and each has its own — so nothing writes
    /// this and nothing should read it. The column goes in a migration
    /// of its own, once no node is still running a version that
    /// selects it.
    pub address: Option<String>,
    /// The tag a push has to carry for this service to care. `None`
    /// means "whatever tag my image reference names".
    pub track_tag: Option<String>,
    /// Whether a push to that tag goes out on its own.
    pub auto_deploy: bool,
    /// The node that asked for this, when it was not this one. `None`
    /// is ours — see migration `0021`.
    pub origin_node_id: Option<String>,
    /// The most memory each of its containers may have, in bytes.
    ///
    /// `None` is no ceiling, which is what every container had before
    /// there were presets. A number here reaches the OCI spec as
    /// `memory.max` with swap turned off — see `runtime::spec`.
    pub memory_limit: Option<u64>,
    /// The most CPU its containers may have, in millicores. `None` is no
    /// ceiling, and **the ceiling is also the reservation** — see
    /// `migrations/0038_cpu_limit.sql` for why there is no second number
    /// for a request.
    pub cpu_millicores: Option<u32>,
    pub kind: Kind,
}

impl Service {
    /// Whether this node is the one that decides about it.
    ///
    /// A service that arrived on an errand is administered from the
    /// node that sent it: nothing here may change it, and the one thing
    /// this node's operator can always do is throw it out. Two nodes
    /// disagreeing about one service is not a conflict anything can
    /// settle, which is the same reason a name belongs to one
    /// authority.
    pub fn is_ours(&self) -> bool {
        self.origin_node_id.is_none()
    }

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
    ///
    /// **Slot 1's**, which is why nothing calls this any more: a service is
    /// *n* copies, and asking a service for "its" container id answered
    /// about one of them. That is what made the memory reading count a
    /// three-copy service once. `Replica::container_id` is the one to ask.
    #[allow(dead_code)]
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
        env: env.iter().cloned().collect(),
        desired_state: DesiredState::Running,
        last_error: None,
        address: None,
        track_tag: None,
        // Made here. A service that came from an errand gets its origin
        // from `set_origin`, which the collector calls and nothing else
        // does — so the default is the honest one and forgetting it
        // cannot silently make a foreign service editable.
        origin_node_id: None,
        // On by default. A node where CI has to be told twice — once
        // to push, once to deploy — is one where the second half gets
        // forgotten and somebody debugs a version that never went out.
        auto_deploy: true,
        // No ceiling, which is what every service had before there
        // were presets. Choosing one is `set_memory_limit`, and a
        // database picks one at creation.
        memory_limit: None,
        cpu_millicores: None,
        // A plain container. `databases::create` calls `set_kind`
        // straight after this, for the same reason `set_origin` is
        // separate: the default is the honest one, and forgetting to
        // say otherwise cannot silently make something managed.
        kind: Kind::Container,
    };

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
                    "this project already has a service called {name:?}"
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
    super::replicas::ensure_here(database, &service.id, 1).await?;

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

/// One service by its slug inside a project.
///
/// Slugs are unique per project, not per node — which is the point of
/// projects — so the lookup needs both halves. This is what every
/// console page has: a URL with the two of them in it.
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

/// The columns `decode` reads, in the order it reads them.
///
/// **One list, because three queries share one decoder.** They were
/// written out three times, and adding a column meant remembering all
/// three — miss one and it is not a compile error but
/// `InvalidColumnIndex(14)` at runtime, from whichever page happened to
/// use that query. Which is exactly what adding `cpu_millicores` did.
const COLUMNS: &str = "\"id\", \"project_id\", \"name\", \"slug\", \"image\", \"env\", \
     \"desired_state\", \"last_error\", \"address\", \"track_tag\", \"auto_deploy\", \
     \"origin_node_id\", \"memory_limit\", \"kind\", \"cpu_millicores\"";

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
    })
}

#[allow(dead_code)]
/// Point a service at a different image.
///
/// Separate from `create` because the reason to call it is that the
/// service already exists — an errand asking for a service that is
/// already there is convergent, and creating a second one under the
/// same name would be the retry making a mess rather than nothing.
pub async fn set_image(
    database: &SqliteDatabase,
    service_id: &str,
    image: &str,
) -> PlatformResult<()> {
    validate_image(image)?;
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

/// Record that another node asked for this service.
///
/// Called by the collector and nothing else. Separate from `create` so
/// that the default is the honest one — a service is this node's until
/// something says otherwise, and forgetting to say so cannot silently
/// make a foreign service editable.
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

/// What a push has to carry, and whether it goes out on its own.
pub async fn set_tracking(
    database: &SqliteDatabase,
    service_id: &str,
    track_tag: Option<&str>,
    auto_deploy: bool,
) -> PlatformResult<()> {
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

/// One service by its id.
///
/// What a caller holding an id has, as against `in_project`, which is
/// what a caller holding a URL has.
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

/// Record that the node manages this service rather than the operator.
///
/// Separate from `create` for the reason `set_origin` is: the default
/// is the honest one. A service that was a plain container for even a
/// moment is one the deploy path would have started without its
/// volume — an empty database that looks like it worked.
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

/// How much memory each of its containers may have.
///
/// Takes effect at the next deployment, like everything else in the OCI
/// spec: a cgroup limit is written when the container is created, and
/// nothing here reaches into a running one to change it. The page says
/// so rather than the operator finding out.
pub async fn set_memory_limit(
    database: &SqliteDatabase,
    service_id: &str,
    bytes: Option<u64>,
) -> PlatformResult<()> {
    if let Some(bytes) = bytes {
        if !super::presets::LADDER.contains(&bytes) {
            return Err(PlatformError::Refused(
                "that is not one of the sizes on offer".into(),
            ));
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

/// Cap how much CPU a service's containers may have, in millicores.
///
/// Refused unless it is one of the rungs on offer, the same shape
/// `set_memory_limit` follows and for the same reason: a field that took
/// any number would be a field somebody types 50 into, which is a
/// container that cannot finish starting.
///
/// Takes effect at the next deployment — a cgroup limit is written into
/// the spec when the container is created, and nothing here reaches into
/// a running one to change it.
pub async fn set_cpu_limit(
    database: &SqliteDatabase,
    service_id: &str,
    millicores: Option<u32>,
) -> PlatformResult<()> {
    if let Some(millicores) = millicores {
        if !super::presets::CPU_LADDER.contains(&millicores) {
            return Err(PlatformError::Refused(
                "that is not one of the CPU sizes on offer".into(),
            ));
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

/// Replace a service's environment.
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
            env: Default::default(),
            desired_state: DesiredState::Running,
            last_error: None,
            address: None,
            track_tag: None,
            origin_node_id: None,
            auto_deploy: true,
            memory_limit: None,
            cpu_millicores: None,
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
        let second = super::super::projects::create(&database, "other")
            .await
            .expect("project")
            .id;

        for project_id in [&first, &second] {
            create(&database, project_id, "api", "nginx:alpine", &[])
                .await
                .expect("created");
        }

        let error = create(&database, &first, "api", "nginx:alpine", &[])
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
        let service = create(&database, &project_id, "api", "nginx:alpine", &[])
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
        let service = create(&database, &project_id, "api", "nginx:alpine", &[])
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
        let service = create(&database, &project_id, "My API", "nginx:alpine", &[])
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
        let service = create(&database, &project_id, "api", "nginx:alpine", &[])
            .await
            .expect("created");
        assert_eq!(service.memory_limit, None);

        let rung = super::super::presets::LADDER[1];
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
        let service = create(&database, &project_id, "api", "nginx:alpine", &[])
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
