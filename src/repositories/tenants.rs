//! Every `SELECT`, `INSERT` and `UPDATE` a [`Tenant`] needs.

use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::SqliteDatabase;

use crate::clock::now_ms;
use crate::entities::tenant::Tenant;
use crate::errors::{PlatformError, PlatformResult};

const COLUMNS: &str = "\"id\", \"name\", \"slug\", \"domain\", \"quota_memory\", \
                       \"quota_cpu\", \"quota_disk\", \"created_at\"";

fn decode(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Tenant> {
    Ok(Tenant {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        domain: row.get(3)?,
        quota_memory: row.get::<_, Option<i64>>(4)?.map(|bytes| bytes as u64),
        quota_cpu: row.get::<_, Option<i64>>(5)?.map(|milli| milli as u32),
        quota_disk: row.get::<_, Option<i64>>(6)?.map(|bytes| bytes as u64),
        created_at: row.get(7)?,
    })
}

pub async fn create(database: &SqliteDatabase, name: &str) -> PlatformResult<Tenant> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 {
        return Err(PlatformError::Refused(
            "an organisation's name is between 1 and 60 characters".into(),
        ));
    }
    let slug = crate::values::slug::Slug::of(name).as_str().to_string();
    if slug.is_empty() {
        return Err(PlatformError::Refused(
            "that name has nothing a hostname can be built from — use letters or digits".into(),
        ));
    }

    let tenant = Tenant {
        id: format!("tnt-{}", wabot::prelude::password::generate(12)),
        name: name.to_string(),
        slug,
        domain: None,
        quota_memory: None,
        quota_cpu: None,
        quota_disk: None,
        created_at: now_ms(),
    };

    let row = tenant.clone();
    let taken = name.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO tenant (\"id\", \"name\", \"slug\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4)",
                (row.id, row.name, row.slug, row.created_at),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| {
            // The unique index is the only thing that can fail here, and
            // "constraint failed" is not something an operator acts on.
            if error.to_string().contains("UNIQUE") {
                PlatformError::Refused(format!(
                    "an organisation already uses the name {taken:?} — pick another"
                ))
            } else {
                PlatformError::Storage(error)
            }
        })?;
    Ok(tenant)
}

pub async fn all(database: &SqliteDatabase) -> PlatformResult<Vec<Tenant>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM tenant ORDER BY \"created_at\" ASC"
                ))?
                .query_map([], decode)?
                .collect()
        })
        .await?)
}

pub async fn find(database: &SqliteDatabase, id: &str) -> PlatformResult<Option<Tenant>> {
    let id = id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM tenant WHERE \"id\" = ?1 OR \"slug\" = ?1"),
                    [id],
                    decode,
                )
                .optional()
        })
        .await?)
}

/// The tenant whose console answers to this hostname.
///
/// What the route table is built from. `None` for a name that is not a
/// tenant's, which is every name until somebody sets a domain.
pub async fn by_domain(database: &SqliteDatabase, domain: &str) -> PlatformResult<Option<Tenant>> {
    let domain = domain.trim().to_lowercase();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM tenant WHERE lower(\"domain\") = ?1"),
                    [domain],
                    decode,
                )
                .optional()
        })
        .await?)
}

/// Set what its console answers to, and what its names hang off.
pub async fn set_domain(
    database: &SqliteDatabase,
    id: &str,
    domain: Option<&str>,
) -> PlatformResult<()> {
    let domain = domain.map(|domain| domain.trim().to_lowercase());
    let (id, domain) = (id.to_string(), domain.filter(|domain| !domain.is_empty()));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE tenant SET \"domain\" = ?2 WHERE \"id\" = ?1",
                (id, domain),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Set the ceiling. `None` on a resource is no ceiling on it.
pub async fn set_quota(
    database: &SqliteDatabase,
    id: &str,
    memory: Option<u64>,
    cpu: Option<u32>,
    disk: Option<u64>,
) -> PlatformResult<()> {
    let (id, memory, cpu, disk) = (
        id.to_string(),
        memory.map(|bytes| bytes as i64),
        cpu.map(i64::from),
        disk.map(|bytes| bytes as i64),
    );
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE tenant SET \"quota_memory\" = ?2, \"quota_cpu\" = ?3, \
                 \"quota_disk\" = ?4 WHERE \"id\" = ?1",
                (id, memory, cpu, disk),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Stop knowing about a tenant.
///
/// **Refused while it still owns projects**, and the refusal names the
/// count. Deleting it with them in would take somebody's containers and
/// volumes in one statement — and the projects would be left in a state
/// nothing can act on: not the node's, not anybody's. The same shape
/// `capability::owns` uses to refuse being turned off while the node
/// still owns projects, and for the same reason.
pub async fn delete(database: &SqliteDatabase, id: &str) -> PlatformResult<()> {
    let held = crate::policies::access::of_tenant(database, id)
        .await?
        .len();
    if held > 0 {
        return Err(PlatformError::Refused(format!(
            "that organisation still has {held} project(s) — move or delete them first"
        )));
    }
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM tenant WHERE \"id\" = ?1", [id])?;
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
    async fn a_tenant_is_made_and_found_by_either_name() {
        let database = database().await;
        let tenant = create(&database, "Acme Corp").await.expect("created");
        assert_eq!(tenant.slug, "acme-corp");
        assert_eq!(
            tenant.quota_memory, None,
            "no quota until somebody sets one"
        );

        for key in [&tenant.id, &tenant.slug] {
            assert_eq!(
                find(&database, key)
                    .await
                    .expect("query")
                    .expect("found")
                    .id,
                tenant.id
            );
        }
    }

    /// Two organisations cannot share a slug.
    ///
    /// It is what a `Host` and a name suffix are built from, so two rows
    /// claiming one is not something a request could resolve — and the
    /// refusal says which name, because "constraint failed" is not a
    /// thing somebody acts on.
    #[tokio::test]
    async fn two_organisations_cannot_share_a_name() {
        let database = database().await;
        create(&database, "Acme").await.expect("first");
        let refused = create(&database, "acme").await.expect_err("refused");
        assert!(refused.to_string().contains("already uses the name"));
    }

    /// A tenant with projects in it is not deleted out from under them.
    ///
    /// They would be neither the node's nor anybody's: not editable,
    /// because nothing originates them, and not evictable, because they
    /// are not somebody else's work. Like a foreign key, and the message
    /// names the count so it is a thing somebody can go and do.
    #[tokio::test]
    async fn a_tenant_that_still_has_projects_is_not_deleted() {
        let database = database().await;
        let tenant = create(&database, "Acme").await.expect("created");
        crate::repositories::projects::create_for(&database, "theirs", Some(&tenant.id))
            .await
            .expect("project");

        let refused = delete(&database, &tenant.id).await.expect_err("refused");
        assert!(refused.to_string().contains("1 project(s)"), "{refused}");
        assert!(find(&database, &tenant.id).await.expect("query").is_some());
    }

    /// A domain is what a `Host` is looked up by, so it is matched the
    /// way a hostname is: without case.
    #[tokio::test]
    async fn a_domain_is_found_however_it_is_capitalised() {
        let database = database().await;
        let tenant = create(&database, "Acme").await.expect("created");
        set_domain(&database, &tenant.id, Some("  Acme.Example  "))
            .await
            .expect("domain");

        for asked in ["acme.example", "ACME.EXAMPLE"] {
            assert_eq!(
                by_domain(&database, asked)
                    .await
                    .expect("query")
                    .expect("found")
                    .id,
                tenant.id
            );
        }
        assert_eq!(
            find(&database, &tenant.id)
                .await
                .expect("query")
                .expect("found")
                .domain
                .as_deref(),
            Some("acme.example"),
            "stored the way it is looked up"
        );
    }
}
