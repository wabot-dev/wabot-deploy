//! Every `SELECT`, `INSERT` and `UPDATE` a store, its buckets and their
//! keys need.

use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::bucket_key::{Access, BucketKey};
use crate::domain::entities::object_store::ObjectStore;
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::domain::policies::presets;
use crate::domain::values::bucket_name::BucketName;
use crate::domain::values::cors_origin::CorsOrigin;
use crate::infra::clock::now_ms;

pub const TAKEN: &str = "this store already has a bucket with that name";

pub const NOT_EMPTY: &str = "a bucket is deleted only when it is empty";

pub async fn create(database: &SqliteDatabase, store: &ObjectStore) -> PlatformResult<()> {
    let row = store.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO object_store \
                   (\"service_id\", \"version\", \"rpc_secret\", \"admin_token\", \
                    \"restored_from\", \"domain\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    row.service_id,
                    row.version,
                    row.rpc_secret,
                    row.admin_token,
                    row.restored_from,
                    row.domain,
                    now_ms(),
                ),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Keep a store another node owns, as its owner last described it, and
/// say whether anything Garage starts from changed: its version, one of
/// its secrets or the domain its buckets are named under, which only a new
/// container picks up.
pub async fn keep(database: &SqliteDatabase, store: &ObjectStore) -> PlatformResult<bool> {
    let row = store.clone();
    Ok(database
        .write(move |connection| {
            type Held = (String, String, String, Option<String>);
            let held: Option<Held> = connection
                .query_row(
                    "SELECT \"version\", \"rpc_secret\", \"admin_token\", \"domain\" \
                       FROM object_store WHERE \"service_id\" = ?1",
                    [&row.service_id],
                    |found| Ok((found.get(0)?, found.get(1)?, found.get(2)?, found.get(3)?)),
                )
                .optional()?;
            let wanted = (
                row.version.clone(),
                row.rpc_secret.clone(),
                row.admin_token.clone(),
                row.domain.clone(),
            );
            if held.as_ref() == Some(&wanted) {
                return Ok(false);
            }
            connection.execute(
                "INSERT INTO object_store \
                   (\"service_id\", \"version\", \"rpc_secret\", \"admin_token\", \"domain\", \
                    \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT (\"service_id\") DO UPDATE SET \"version\" = ?2, \
                   \"rpc_secret\" = ?3, \"admin_token\" = ?4, \"domain\" = ?5",
                (
                    &row.service_id,
                    wanted.0,
                    wanted.1,
                    wanted.2,
                    wanted.3,
                    now_ms(),
                ),
            )?;
            Ok(true)
        })
        .await?)
}

/// Make a store's buckets and keys exactly these, matched by bucket name
/// and by access key: the ids are each node's own. One transaction, so a
/// convergence reading half-way cannot see a bucket without its keys and
/// delete them from Garage.
///
/// No room is checked here, unlike `create_bucket`: the owner checked it,
/// and a node that refused what its owner accepted would hold a store that
/// is not the one described.
pub async fn replace_contents(
    database: &SqliteDatabase,
    service_id: &str,
    buckets: Vec<Bucket>,
    keys: Vec<(String, BucketKey)>,
) -> PlatformResult<()> {
    let service_id = service_id.to_string();
    database
        .write(move |connection| {
            let transaction = connection.unchecked_transaction()?;
            let mut ids: std::collections::BTreeMap<String, String> = Default::default();
            for bucket in &buckets {
                let held: Option<String> = transaction
                    .query_row(
                        "SELECT \"id\" FROM bucket WHERE \"service_id\" = ?1 AND \"name\" = ?2",
                        [&service_id, bucket.name.as_str()],
                        |found| found.get(0),
                    )
                    .optional()?;
                let id = match held {
                    Some(id) => {
                        transaction.execute(
                            "UPDATE bucket SET \"max_size\" = ?2, \"public_hostname\" = ?3, \
                                    \"cors_origins\" = ?4, \"expire_days\" = ?5, \
                                    \"abort_uploads_days\" = ?6, \"public\" = ?7 \
                              WHERE \"id\" = ?1",
                            (
                                &id,
                                bucket.max_size as i64,
                                &bucket.public_hostname,
                                origins_text(&bucket.cors_origins),
                                bucket.expire_days,
                                bucket.abort_uploads_days,
                                bucket.public,
                            ),
                        )?;
                        id
                    }
                    None => {
                        transaction.execute(
                            "INSERT INTO bucket \
                               (\"id\", \"service_id\", \"name\", \"max_size\", \"public_hostname\", \
                                \"cors_origins\", \"expire_days\", \"abort_uploads_days\", \"public\", \
                                \"created_at\") \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                            (
                                &bucket.id,
                                &service_id,
                                bucket.name.as_str(),
                                bucket.max_size as i64,
                                &bucket.public_hostname,
                                origins_text(&bucket.cors_origins),
                                bucket.expire_days,
                                bucket.abort_uploads_days,
                                bucket.public,
                                now_ms(),
                            ),
                        )?;
                        bucket.id.clone()
                    }
                };
                ids.insert(bucket.name.as_str().to_string(), id);
            }
            let named: Vec<String> = ids.values().cloned().collect();
            let held: Vec<String> = {
                let mut statement =
                    transaction.prepare("SELECT \"id\" FROM bucket WHERE \"service_id\" = ?1")?;
                let rows: wabot::sqlite::rusqlite::Result<Vec<String>> = statement
                    .query_map([&service_id], |found| found.get(0))?
                    .collect();
                rows?
            };
            for id in held.iter().filter(|id| !named.contains(id)) {
                transaction.execute("DELETE FROM bucket WHERE \"id\" = ?1", [id])?;
            }

            let mut kept = Vec::new();
            for (bucket, key) in &keys {
                let Some(bucket_id) = ids.get(bucket) else {
                    continue;
                };
                let updated = transaction.execute(
                    "UPDATE bucket_key SET \"bucket_id\" = ?2, \"name\" = ?3, \"secret_key\" = ?4, \
                            \"access\" = ?5 \
                      WHERE \"access_key_id\" = ?1",
                    (
                        &key.access_key_id,
                        bucket_id,
                        &key.name,
                        &key.secret_key,
                        key.access.as_str(),
                    ),
                )?;
                if updated == 0 {
                    transaction.execute(
                        "INSERT INTO bucket_key \
                           (\"id\", \"bucket_id\", \"name\", \"access_key_id\", \"secret_key\", \
                            \"access\", \"created_at\") \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        (
                            &key.id,
                            bucket_id,
                            &key.name,
                            &key.access_key_id,
                            &key.secret_key,
                            key.access.as_str(),
                            now_ms(),
                        ),
                    )?;
                }
                kept.push(key.access_key_id.clone());
            }
            let held: Vec<String> = {
                let mut statement = transaction.prepare(
                    "SELECT k.\"access_key_id\" FROM bucket_key k \
                       JOIN bucket b ON b.\"id\" = k.\"bucket_id\" WHERE b.\"service_id\" = ?1",
                )?;
                let rows: wabot::sqlite::rusqlite::Result<Vec<String>> = statement
                    .query_map([&service_id], |found| found.get(0))?
                    .collect();
                rows?
            };
            for access_key_id in held.iter().filter(|id| !kept.contains(id)) {
                transaction.execute(
                    "DELETE FROM bucket_key WHERE \"access_key_id\" = ?1",
                    [access_key_id],
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Replace what the node running this store last measured in its buckets.
pub async fn set_measured(
    database: &SqliteDatabase,
    service_id: &str,
    measured: &[crate::domain::entities::bucket::Measured],
) -> PlatformResult<()> {
    let (service_id, measured) = (service_id.to_string(), measured.to_vec());
    database
        .write(move |connection| {
            let transaction = connection.unchecked_transaction()?;
            transaction.execute(
                "DELETE FROM bucket_usage WHERE \"service_id\" = ?1",
                [&service_id],
            )?;
            let now = now_ms();
            for bucket in measured {
                transaction.execute(
                    "INSERT INTO bucket_usage \
                       (\"service_id\", \"name\", \"objects\", \"bytes\", \"unfinished_uploads\", \
                        \"measured_at\") \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (
                        &service_id,
                        bucket.name,
                        bucket.objects as i64,
                        bucket.bytes as i64,
                        bucket.unfinished_uploads as i64,
                        now,
                    ),
                )?;
            }
            transaction.commit()?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn measured(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Vec<crate::domain::entities::bucket::Measured>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            let mut statement = connection.prepare(
                "SELECT \"name\", \"objects\", \"bytes\", \"unfinished_uploads\" FROM bucket_usage \
                  WHERE \"service_id\" = ?1 ORDER BY \"name\"",
            )?;
            let rows = statement.query_map([service_id], |row| {
                Ok(crate::domain::entities::bucket::Measured {
                    name: row.get(0)?,
                    objects: row.get::<_, i64>(1)? as u64,
                    bytes: row.get::<_, i64>(2)? as u64,
                    unfinished_uploads: row.get::<_, i64>(3)? as u64,
                })
            })?;
            rows.collect()
        })
        .await?)
}

pub async fn set_restored_from(
    database: &SqliteDatabase,
    service_id: &str,
    from: Option<&str>,
) -> PlatformResult<()> {
    let service_id = service_id.to_string();
    let from = from.map(str::to_string);
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE object_store SET \"restored_from\" = ?2 WHERE \"service_id\" = ?1",
                (&service_id, &from),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Option<ObjectStore>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"service_id\", \"version\", \"rpc_secret\", \"admin_token\", \
                            \"restored_from\", \"domain\" \
                       FROM object_store WHERE \"service_id\" = ?1",
                    [service_id],
                    |row| {
                        Ok(ObjectStore {
                            service_id: row.get(0)?,
                            version: row.get(1)?,
                            rpc_secret: row.get(2)?,
                            admin_token: row.get(3)?,
                            domain: row.get(5)?,
                            restored_from: row.get(4)?,
                        })
                    },
                )
                .optional()
        })
        .await?)
}

/// Every public bucket's own name, `<bucket>.<domain>`: the names whose
/// unsigned reads are a bucket's public ones.
pub async fn public_bucket_names(database: &SqliteDatabase) -> PlatformResult<Vec<String>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT b.\"name\" || '.' || s.\"domain\" \
                       FROM bucket b JOIN object_store s ON s.\"service_id\" = b.\"service_id\" \
                      WHERE b.\"public\" = 1 AND s.\"domain\" IS NOT NULL",
                )?
                .query_map([], |row| row.get(0))?
                .collect()
        })
        .await?)
}

pub async fn buckets(database: &SqliteDatabase, service_id: &str) -> PlatformResult<Vec<Bucket>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {BUCKET} FROM bucket WHERE \"service_id\" = ?1 ORDER BY \"name\""
            ))?;
            let rows = statement.query_map([service_id], decode_bucket)?;
            rows.collect()
        })
        .await?)
}

pub async fn bucket(
    database: &SqliteDatabase,
    service_id: &str,
    name: &str,
) -> PlatformResult<Option<Bucket>> {
    let (service_id, name) = (service_id.to_string(), name.to_string());
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!(
                        "SELECT {BUCKET} FROM bucket WHERE \"service_id\" = ?1 AND \"name\" = ?2"
                    ),
                    [service_id, name],
                    decode_bucket,
                )
                .optional()
        })
        .await?)
}

/// What a store's buckets may hold together, against its disk ceiling.
///
/// Here rather than in a command because no caller could legitimately want
/// the sum past the ceiling: Garage has quotas per bucket and none per store,
/// so this is the only thing standing between a full bucket and a disk other
/// services share.
fn fits(
    connection: &wabot::sqlite::rusqlite::Connection,
    service_id: &str,
    except: &str,
    asked: u64,
) -> wabot::sqlite::rusqlite::Result<Result<(), String>> {
    let ceiling: Option<i64> = connection
        .query_row(
            "SELECT \"disk_limit\" FROM service WHERE \"id\" = ?1",
            [service_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let Some(ceiling) = ceiling.map(|bytes| bytes as u64) else {
        return Ok(Err(
            "this store has no disk ceiling, and its buckets are sized against it".into(),
        ));
    };
    let others: i64 = connection.query_row(
        "SELECT COALESCE(SUM(\"max_size\"), 0) FROM bucket \
          WHERE \"service_id\" = ?1 AND \"id\" != ?2",
        [service_id, except],
        |row| row.get(0),
    )?;
    let others = others as u64;
    if others.saturating_add(asked) > ceiling {
        return Ok(Err(format!(
            "{} would take this store past its disk ceiling of {}: its other buckets \
             already hold up to {}, which leaves {}",
            presets::size_field(asked),
            presets::size_field(ceiling),
            presets::size_field(others),
            presets::size_field(ceiling.saturating_sub(others)),
        )));
    }
    Ok(Ok(()))
}

pub async fn create_bucket(database: &SqliteDatabase, bucket: &Bucket) -> PlatformResult<()> {
    let row = bucket.clone();
    let refused = database
        .write(move |connection| {
            let taken: bool = connection
                .query_row(
                    "SELECT 1 FROM bucket WHERE \"service_id\" = ?1 AND \"name\" = ?2",
                    [&row.service_id, row.name.as_str()],
                    |_| Ok(true),
                )
                .optional()?
                .unwrap_or(false);
            if taken {
                return Ok(Some(TAKEN.to_string()));
            }
            if let Err(reason) = fits(connection, &row.service_id, &row.id, row.max_size)? {
                return Ok(Some(reason));
            }
            connection.execute(
                "INSERT INTO bucket \
                   (\"id\", \"service_id\", \"name\", \"max_size\", \"public_hostname\", \
                    \"cors_origins\", \"expire_days\", \"abort_uploads_days\", \"public\", \
                    \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                (
                    &row.id,
                    &row.service_id,
                    row.name.as_str(),
                    row.max_size as i64,
                    row.public_hostname,
                    origins_text(&row.cors_origins),
                    row.expire_days,
                    row.abort_uploads_days,
                    row.public,
                    now_ms(),
                ),
            )?;
            Ok(None)
        })
        .await?;
    match refused {
        Some(reason) => Err(PlatformError::Refused(reason)),
        None => Ok(()),
    }
}

pub async fn set_max_size(
    database: &SqliteDatabase,
    bucket: &Bucket,
    max_size: u64,
) -> PlatformResult<()> {
    if max_size == 0 {
        return Err(PlatformError::Refused(
            crate::domain::entities::bucket::NO_ROOM.into(),
        ));
    }
    let (id, service_id) = (bucket.id.clone(), bucket.service_id.clone());
    let refused = database
        .write(move |connection| {
            if let Err(reason) = fits(connection, &service_id, &id, max_size)? {
                return Ok(Some(reason));
            }
            connection.execute(
                "UPDATE bucket SET \"max_size\" = ?2 WHERE \"id\" = ?1",
                (&id, max_size as i64),
            )?;
            Ok(None)
        })
        .await?;
    match refused {
        Some(reason) => Err(PlatformError::Refused(reason)),
        None => Ok(()),
    }
}

/// The most a store's buckets may hold together, for the form that would
/// lower its ceiling under it.
pub async fn reserved(database: &SqliteDatabase, service_id: &str) -> PlatformResult<u64> {
    let service_id = service_id.to_string();
    let sum: i64 = database
        .read(move |connection| {
            connection.query_row(
                "SELECT COALESCE(SUM(\"max_size\"), 0) FROM bucket WHERE \"service_id\" = ?1",
                [service_id],
                |row| row.get(0),
            )
        })
        .await?;
    Ok(sum as u64)
}

pub async fn set_access_rules(database: &SqliteDatabase, bucket: &Bucket) -> PlatformResult<()> {
    let row = bucket.clone();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE bucket SET \"cors_origins\" = ?2, \"expire_days\" = ?3, \
                        \"abort_uploads_days\" = ?4 \
                  WHERE \"id\" = ?1",
                (
                    &row.id,
                    origins_text(&row.cors_origins),
                    row.expire_days,
                    row.abort_uploads_days,
                ),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn set_public(
    database: &SqliteDatabase,
    id: &str,
    public: bool,
    hostname: Option<&str>,
) -> PlatformResult<()> {
    let id = id.to_string();
    let hostname = hostname.map(str::to_string);
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE bucket SET \"public\" = ?2, \"public_hostname\" = ?3 WHERE \"id\" = ?1",
                (&id, public, &hostname),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn delete_bucket(database: &SqliteDatabase, id: &str) -> PlatformResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM bucket WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn keys(database: &SqliteDatabase, bucket_id: &str) -> PlatformResult<Vec<BucketKey>> {
    let bucket_id = bucket_id.to_string();
    Ok(database
        .read(move |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {KEY} FROM bucket_key WHERE \"bucket_id\" = ?1 ORDER BY \"created_at\""
            ))?;
            let rows = statement.query_map([bucket_id], decode_key)?;
            rows.collect()
        })
        .await?)
}

/// Every key of every bucket in a store: what convergence leaves in Garage,
/// and nothing else.
pub async fn keys_of_store(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Vec<BucketKey>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {KEY_QUALIFIED} FROM bucket_key k JOIN bucket b ON b.\"id\" = k.\"bucket_id\" \
                  WHERE b.\"service_id\" = ?1 ORDER BY k.\"created_at\""
            ))?;
            let rows = statement.query_map([service_id], decode_key)?;
            rows.collect()
        })
        .await?)
}

pub async fn create_key(database: &SqliteDatabase, key: &BucketKey) -> PlatformResult<()> {
    let row = key.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO bucket_key \
                   (\"id\", \"bucket_id\", \"name\", \"access_key_id\", \"secret_key\", \
                    \"access\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    row.id,
                    row.bucket_id,
                    row.name,
                    row.access_key_id,
                    row.secret_key,
                    row.access.as_str(),
                    now_ms(),
                ),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn delete_key(database: &SqliteDatabase, id: &str) -> PlatformResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM bucket_key WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

fn origins_text(origins: &[CorsOrigin]) -> String {
    origins
        .iter()
        .map(CorsOrigin::as_str)
        .collect::<Vec<_>>()
        .join("\n")
}

const BUCKET: &str = "\"id\", \"service_id\", \"name\", \"max_size\", \"public_hostname\", \
                      \"cors_origins\", \"expire_days\", \"abort_uploads_days\", \"public\"";

fn decode_bucket(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Bucket> {
    let name: String = row.get(2)?;
    Ok(Bucket {
        id: row.get(0)?,
        service_id: row.get(1)?,
        name: BucketName::stored(&name),
        max_size: row.get::<_, i64>(3)? as u64,
        public: row.get(8)?,
        public_hostname: row.get(4)?,
        cors_origins: CorsOrigin::parse_lines(&row.get::<_, String>(5)?).unwrap_or_default(),
        expire_days: row.get(6)?,
        abort_uploads_days: row.get(7)?,
    })
}

const KEY: &str = "\"id\", \"bucket_id\", \"name\", \"access_key_id\", \"secret_key\", \"access\"";

const KEY_QUALIFIED: &str = "k.\"id\", k.\"bucket_id\", k.\"name\", k.\"access_key_id\", \
                             k.\"secret_key\", k.\"access\"";

fn decode_key(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<BucketKey> {
    Ok(BucketKey {
        id: row.get(0)?,
        bucket_id: row.get(1)?,
        name: row.get(2)?,
        access_key_id: row.get(3)?,
        secret_key: row.get(4)?,
        access: Access::parse(&row.get::<_, String>(5)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> (SqliteDatabase, crate::domain::entities::service::Service) {
        use crate::infra::repositories::{projects, services};
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "shop").await.expect("project");
        let service = services::create(
            &database,
            &project.id,
            "files",
            &crate::domain::policies::garage::image_for("2").expect("image"),
            &[],
        )
        .await
        .expect("service");
        services::set_disk_limit(&database, &service.id, Some(1024 * 1024 * 1024))
            .await
            .expect("disk");
        create(
            &database,
            &ObjectStore {
                service_id: service.id.clone(),
                version: "2".into(),
                rpc_secret: "ab".repeat(32),
                admin_token: "token".into(),
                domain: Some("files.shop.example.com".into()),
                restored_from: None,
            },
        )
        .await
        .expect("store");
        (database, service)
    }

    fn bucket(service_id: &str, name: &str, max_size: u64) -> Bucket {
        Bucket {
            id: format!("bkt-{name}"),
            service_id: service_id.to_string(),
            name: BucketName::stored(name),
            max_size,
            public: false,
            public_hostname: None,
            cors_origins: Vec::new(),
            expire_days: None,
            abort_uploads_days: None,
        }
    }

    fn key(access_key_id: &str, name: &str) -> BucketKey {
        BucketKey {
            id: format!("bk-{access_key_id}"),
            bucket_id: String::new(),
            name: name.to_string(),
            access_key_id: access_key_id.to_string(),
            secret_key: "s".repeat(64),
            access: Access::ReadWrite,
        }
    }

    /// What a node keeping somebody's store holds is exactly what the last
    /// errand said, matched by name: a bucket keeps its row and the objects
    /// Garage holds under it, and what the owner removed is removed.
    #[tokio::test]
    async fn a_stores_contents_become_exactly_what_its_owner_described() {
        let (database, service) = store().await;
        replace_contents(
            &database,
            &service.id,
            vec![
                bucket(&service.id, "uploads", 100),
                bucket(&service.id, "logs", 100),
            ],
            vec![
                ("uploads".into(), key("GK1", "web")),
                ("logs".into(), key("GK2", "shipper")),
            ],
        )
        .await
        .expect("first");
        let uploads = bucket_named(&database, &service.id, "uploads").await;

        let mut grown = bucket(&service.id, "uploads", 500);
        grown.id = "bkt-another-id".into();
        replace_contents(
            &database,
            &service.id,
            vec![grown],
            vec![("uploads".into(), key("GK1", "web, renamed"))],
        )
        .await
        .expect("second");

        let held = buckets(&database, &service.id).await.expect("buckets");
        assert_eq!(held.len(), 1, "{held:?}");
        assert_eq!(held[0].id, uploads.id, "a bucket is matched by its name");
        assert_eq!(held[0].max_size, 500);
        let keys = keys_of_store(&database, &service.id).await.expect("keys");
        assert_eq!(
            keys.iter()
                .map(|key| (key.access_key_id.as_str(), key.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("GK1", "web, renamed")]
        );
    }

    async fn bucket_named(database: &SqliteDatabase, service_id: &str, name: &str) -> Bucket {
        buckets(database, service_id)
            .await
            .expect("buckets")
            .into_iter()
            .find(|bucket| bucket.name.as_str() == name)
            .expect("there")
    }

    /// A new secret or version is only read by a new container, and a store
    /// restarted for every errand would cut every upload in flight.
    #[tokio::test]
    async fn keeping_a_store_says_whether_what_garage_starts_from_changed() {
        let (database, service) = store().await;
        let held = of_service(&database, &service.id)
            .await
            .expect("read")
            .expect("row");
        assert!(!keep(&database, &held).await.expect("same"));
        let rotated = ObjectStore {
            admin_token: "another".into(),
            ..held
        };
        assert!(keep(&database, &rotated).await.expect("rotated"));
        assert!(!keep(&database, &rotated).await.expect("again"));
        let renamed = ObjectStore {
            domain: Some("s3.example.com".into()),
            ..rotated
        };
        assert!(keep(&database, &renamed).await.expect("renamed"));
    }
}
