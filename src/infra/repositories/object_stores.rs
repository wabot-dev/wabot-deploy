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
                   (\"service_id\", \"version\", \"rpc_secret\", \"admin_token\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (
                    row.service_id,
                    row.version,
                    row.rpc_secret,
                    row.admin_token,
                    now_ms(),
                ),
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
                    "SELECT \"service_id\", \"version\", \"rpc_secret\", \"admin_token\" \
                       FROM object_store WHERE \"service_id\" = ?1",
                    [service_id],
                    |row| {
                        Ok(ObjectStore {
                            service_id: row.get(0)?,
                            version: row.get(1)?,
                            rpc_secret: row.get(2)?,
                            admin_token: row.get(3)?,
                        })
                    },
                )
                .optional()
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
                    \"cors_origins\", \"expire_days\", \"abort_uploads_days\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                (
                    &row.id,
                    &row.service_id,
                    row.name.as_str(),
                    row.max_size as i64,
                    row.public_hostname,
                    origins_text(&row.cors_origins),
                    row.expire_days,
                    row.abort_uploads_days,
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

pub async fn set_public_hostname(
    database: &SqliteDatabase,
    id: &str,
    hostname: Option<&str>,
) -> PlatformResult<()> {
    let id = id.to_string();
    let hostname = hostname.map(str::to_string);
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE bucket SET \"public_hostname\" = ?2 WHERE \"id\" = ?1",
                (&id, &hostname),
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
                      \"cors_origins\", \"expire_days\", \"abort_uploads_days\"";

fn decode_bucket(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Bucket> {
    let name: String = row.get(2)?;
    Ok(Bucket {
        id: row.get(0)?,
        service_id: row.get(1)?,
        name: BucketName::stored(&name),
        max_size: row.get::<_, i64>(3)? as u64,
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
