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
    /// The node that asked for this, when it was not this one. `None`
    /// is ours — see migration `0021`.
    pub origin_node_id: Option<String>,
    /// The tenant it belongs to. `None` is the node's own — the same
    /// spelling the two above use, so a row written before tenants
    /// existed still means what it meant. See migration `0054`.
    pub tenant_id: Option<String>,
}

impl Project {
    /// Whether this node is the one that decides about it.
    ///
    /// A project that arrived on an errand is administered from the
    /// node that sent it: nothing here may change it, and the one thing
    /// this node's operator can always do is throw it out.
    ///
    /// Read by the capabilities form, which refuses to let a node stop
    /// being an owner while any of these are still here — a project of
    /// this node's own on a node that originates nothing would be
    /// neither editable nor evictable. `crate::entities::service::Service::is_ours` is
    /// the same check one level down, and every service mutation goes
    /// through it.
    pub fn is_ours(&self) -> bool {
        self.origin_node_id.is_none()
    }

    /// Whether this is the machine's own rather than an organisation's.
    ///
    /// A question rather than a field read, so a controller that has no
    /// business knowing what an organisation is does not have to name
    /// one — and so deleting tenancy leaves this answering `true` for
    /// everything, which is what it meant before organisations existed.
    pub fn is_the_nodes_own(&self) -> bool {
        self.tenant_id.is_none()
    }
}

pub async fn create(database: &SqliteDatabase, name: &str) -> PlatformResult<Project> {
    create_for(database, name, None).await
}

/// The same, inside a tenant.
///
/// `None` is the node's own, which is what `create` means and what every
/// project made before tenants existed is.
pub async fn create_for(
    database: &SqliteDatabase,
    name: &str,
    tenant_id: Option<&str>,
) -> PlatformResult<Project> {
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
        // Made here. A project that came from an errand is written by
        // `adopt`, which is the only way one gets an origin.
        origin_node_id: None,
        tenant_id: tenant_id.map(str::to_string),
    };

    let row = project.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO project (\"id\", \"name\", \"slug\", \"created_at\", \"tenant_id\") \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (row.id, row.name, row.slug, row.created_at, row.tenant_id),
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

/// Record that another node asked for this project.
///
/// Same reasoning as `services::set_origin`: separate from `create` so
/// that a project is this node's own unless something said otherwise.
pub async fn set_origin(database: &SqliteDatabase, id: &str, node_id: &str) -> PlatformResult<()> {
    let (id, node_id) = (id.to_string(), node_id.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE project SET \"origin_node_id\" = ?2 WHERE \"id\" = ?1",
                (id, node_id),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// The domain this project's names hang off.
///
/// Its organisation's when it has one, and this node's otherwise —
/// which is every project the node owns itself, and every project that
/// existed before organisations did.
///
/// **Here rather than in `tenants`, and that is the point.** Its three
/// callers ask about a *project*, which is a thing this node has with
/// or without organisations — so none of them has to name one, and
/// deleting tenancy simplifies this function instead of touching them.
///
/// **One function because there are three callers and they must not
/// disagree**: the name a page proposes, the name a certificate is
/// ordered for, and the name written into a container's `/etc/hosts`.
/// Two of those drifting once already produced a client dialling a name
/// no certificate held — see `certificate_names`.
///
/// It does **not** override `database.owner_domain`. That column answers
/// a different question — whose name is this copy known by, when the
/// copy is held for somebody else — and it wins wherever it is set: a
/// database placed on another node keeps the name its owner gave it, and
/// that owner may be an organisation on a third machine entirely.
pub async fn domain_for(
    database: &SqliteDatabase,
    project: &Project,
    node_domain: Option<&str>,
) -> Option<String> {
    let theirs = match &project.tenant_id {
        Some(tenant_id) => super::tenants::find(database, tenant_id)
            .await
            .ok()
            .flatten()
            .and_then(|tenant| tenant.domain),
        None => None,
    };
    theirs.or_else(|| node_domain.map(str::to_string))
}

pub async fn all(database: &SqliteDatabase) -> PlatformResult<Vec<Project>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"name\", \"slug\", \"created_at\", \"origin_node_id\", \"tenant_id\" \
                     FROM project \
                     ORDER BY \"created_at\" ASC",
                )?
                .query_map([], |row| {
                    Ok(Project {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        slug: row.get(2)?,
                        created_at: row.get(3)?,
                        origin_node_id: row.get(4)?,
                        tenant_id: row.get(5)?,
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
                    "SELECT \"id\", \"name\", \"slug\", \"created_at\", \"origin_node_id\", \"tenant_id\" \
                     FROM project \
                     WHERE \"id\" = ?1 OR \"slug\" = ?1",
                    [id],
                    |row| {
                        Ok(Project {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            slug: row.get(2)?,
                            created_at: row.get(3)?,
                            origin_node_id: row.get(4)?,
                            tenant_id: row.get(5)?,
                        })
                    },
                )
                .optional()
        })
        .await?)
}

/// The project's slot in the address space, allocating one if it has
/// none.
///
/// Allocated here rather than at creation: a project that never runs
/// anything should not be holding one of the 254 subnets. The unique
/// index is what makes it safe when two deploys race — the loser sees
/// a constraint failure and reads back the winner's value, which is
/// the correct outcome rather than two projects on one bridge.
pub async fn ensure_network_index(database: &SqliteDatabase, id: &str) -> PlatformResult<u8> {
    if let Some(index) = network_index(database, id).await? {
        return Ok(index);
    }

    let project = id.to_string();
    let taken: Vec<i64> = database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"network_index\" FROM project \
                     WHERE \"network_index\" IS NOT NULL ORDER BY 1",
                )?
                .query_map([], |row| row.get(0))?
                .collect()
        })
        .await?;

    // The lowest free slot, so deleting a project makes its subnet
    // available again rather than walking the space until it runs out.
    let next = (1i64..=254)
        .find(|candidate| !taken.contains(candidate))
        .ok_or_else(|| {
            PlatformError::Refused(
                "every subnet in 10.42.0.0/16 is allocated — that is 254 projects with \
                 something deployed"
                    .into(),
            )
        })?;

    let claimed = database
        .write(move |connection| {
            connection.execute(
                "UPDATE project SET \"network_index\" = ?2 \
                 WHERE \"id\" = ?1 AND \"network_index\" IS NULL",
                (project, next),
            )
        })
        .await;

    match claimed {
        Ok(1) => Ok(next as u8),
        // Either somebody else claimed this project's slot first, or
        // the index was taken between the read and the write. Both are
        // answered by looking again.
        _ => network_index(database, id)
            .await?
            .ok_or_else(|| PlatformError::Refused("could not allocate a subnet".into())),
    }
}

async fn network_index(database: &SqliteDatabase, id: &str) -> PlatformResult<Option<u8>> {
    let id = id.to_string();
    let value: Option<Option<i64>> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"network_index\" FROM project WHERE \"id\" = ?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()
        })
        .await?;
    Ok(value.flatten().map(|index| index as u8))
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
        crate::commands::create_service::CreateService {
            project_id: &project.id,
            name: "api",
            image: "docker.io/library/nginx:alpine",
            env: &[],
        }
        .run(&database)
        .await
        .expect("service");

        delete(&database, &project.id).await.expect("delete");

        assert!(
            crate::repositories::services::all(&database, Some(&project.id))
                .await
                .expect("query")
                .is_empty()
        );
    }
}
