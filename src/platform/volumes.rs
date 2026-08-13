//! Storage that outlives the container.
//!
//! ## What a container had instead
//!
//! Nothing. `containers::run` removes whatever is under the id before
//! it starts, and removing a container removes its snapshot — so what a
//! container wrote is gone at the next deployment. Right for a service
//! that starts from its image, and total loss for anything holding
//! data.
//!
//! ## The row is the service's, the directory is the replica's
//!
//! Where to mount and what to call it is one answer for every copy: the
//! same image with the same layout. The bytes are not. Two copies of a
//! database on one node are two databases, and a directory they shared
//! would be two servers writing one data directory, which corrupts it
//! in seconds rather than eventually.
//!
//! ## The directory is derived, never stored
//!
//! `<data_dir>/volumes/<container id>/<name>` — reconstructible from
//! the rows alone, which is the same rule `Replica::container_id`
//! follows and for the same reason: what cleans up after a crash starts
//! from the rows and asks the disk what it has.
//!
//! Nothing here deletes anything. A directory whose rows went away is
//! an orphan somebody can still recover from; a directory this module
//! removed on its own is not. Discarding one is [`discard`], and the
//! only caller is a confirmation somebody typed.

use std::path::{Path, PathBuf};

use serde::Serialize;
use wabot::sqlite::SqliteDatabase;

use super::{now_ms, slugify, PlatformError, PlatformResult};

/// One mount point a service keeps its data in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Volume {
    pub id: String,
    pub service_id: String,
    /// A slug. The last component of the directory on the node.
    pub name: String,
    /// Where it appears inside the container.
    pub path: String,
}

/// Where every volume on this node lives, under the data directory.
pub fn root(data_dir: &Path) -> PathBuf {
    data_dir.join("volumes")
}

/// The directory holding one copy's data for one volume.
///
/// Keyed on the **container id**, which already carries the project,
/// the service and the slot — so an operator reading `ctr containers
/// ls` and `ls volumes/` sees the same names on both sides, and slot 1
/// keeps the name it had before there were slots.
pub fn directory(data_dir: &Path, container_id: &str, name: &str) -> PathBuf {
    root(data_dir).join(container_id).join(name)
}

/// Make sure a copy's directory exists, and hand back its path.
///
/// Convergent: it asks whether the directory is there, not whether it
/// has ever made one. A deployment is the only caller, and a deployment
/// runs again for every reason.
pub fn ensure(data_dir: &Path, container_id: &str, name: &str) -> std::io::Result<PathBuf> {
    let path = directory(data_dir, container_id, name);
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

/// Throw away everything one copy stored.
///
/// The whole container's directory, not one volume of it: the caller is
/// somebody deleting the thing that owned them.
///
/// A directory that is not there is already discarded — the same
/// tolerance every teardown in `runtime::containers` has, and for the
/// same reason: the usual cause of being here is that something else
/// half-failed.
pub fn discard(data_dir: &Path, container_id: &str) -> std::io::Result<()> {
    let path = root(data_dir).join(container_id);
    match std::fs::remove_dir_all(&path) {
        Ok(()) => {
            tracing::info!(directory = %path.display(), "discarded a copy's data");
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Directories under `volumes/` that no live container id claims.
///
/// Reported rather than removed. A node that deletes storage it does
/// not recognise is a node that destroys the one copy of something the
/// moment a row is missing for a reason nobody has understood yet —
/// which is the same rule reconciliation follows about containers.
pub fn orphans(data_dir: &Path, live: &[String]) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root(data_dir)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            !live.contains(&name)
        })
        .map(|entry| entry.path())
        .collect()
}

/// Declare that a service keeps data at `path`.
pub async fn create(
    database: &SqliteDatabase,
    service_id: &str,
    name: &str,
    path: &str,
) -> PlatformResult<Volume> {
    let name = slugify(name);
    if name.is_empty() || name.len() > 32 {
        return Err(PlatformError::Refused(
            "a volume name is 1 to 32 characters a directory can hold".into(),
        ));
    }
    let path = validate_path(path)?;

    let volume = Volume {
        id: format!("vol-{}", wabot::prelude::password::generate(12)),
        service_id: service_id.to_string(),
        name,
        path,
    };

    let row = volume.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO volume (\"id\", \"service_id\", \"name\", \"path\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (row.id, row.service_id, row.name, row.path, now_ms()),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| match error.to_string().contains("UNIQUE") {
            true => PlatformError::Refused(format!(
                "this service already keeps data at {:?}",
                volume.path
            )),
            false => PlatformError::Storage(error),
        })?;

    Ok(volume)
}

/// Where a container may not be given a directory.
///
/// Every one of these is a mount the container needs to be a container
/// — see `runtime::spec` — and a volume is appended *after* them, so it
/// would win. Losing `/proc` is a container that does not start;
/// losing `/etc/resolv.conf` is one that starts and cannot resolve
/// anything, which is worse because it looks like the image's fault.
const RESERVED: [&str; 9] = [
    "/",
    "/proc",
    "/dev",
    "/dev/pts",
    "/dev/shm",
    "/dev/mqueue",
    "/sys",
    "/etc/resolv.conf",
    // The node writes this one too, from the rows — see `deploy::hosts`.
    // A volume over it is a container that can reach nothing by name.
    "/etc/hosts",
];

/// A destination the node can mount over without breaking the
/// container.
///
/// Trailing slashes are trimmed so `/data` and `/data/` are one path
/// rather than two rows the unique index would let through.
fn validate_path(path: &str) -> PlatformResult<String> {
    let path = path.trim().trim_end_matches('/');
    let path = match path.is_empty() {
        true => "/",
        false => path,
    };

    if !path.starts_with('/') {
        return Err(PlatformError::Refused(format!(
            "a mount point is an absolute path: {path:?} — try /var/lib/postgresql/data"
        )));
    }
    // `..` is refused rather than resolved. The destination is
    // interpreted inside the container's own root by the runtime, so a
    // traversal cannot reach the node's filesystem — but a path nobody
    // can predict from reading it is one nobody can audit either.
    if path.split('/').any(|part| part == "..") {
        return Err(PlatformError::Refused(
            "a mount point has no `..` in it".into(),
        ));
    }
    if RESERVED.contains(&path) {
        return Err(PlatformError::Refused(format!(
            "{path} is one of the mounts a container needs to run, so nothing may be \
             mounted over it"
        )));
    }
    Ok(path.to_string())
}

pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Vec<Volume>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            let mut statement = connection.prepare(
                "SELECT \"id\", \"service_id\", \"name\", \"path\" FROM volume \
                 WHERE \"service_id\" = ?1 ORDER BY \"path\"",
            )?;
            let volumes: wabot::sqlite::rusqlite::Result<Vec<Volume>> =
                statement.query_map([service_id], decode)?.collect();
            volumes
        })
        .await?)
}

/// Stop mounting this, and leave what it holds on the disk.
///
/// The directory stays. Removing the row is a change to what the next
/// deployment mounts; removing the bytes is a decision with no way
/// back, and the two do not belong in one call.
#[allow(dead_code)]
pub async fn delete(database: &SqliteDatabase, id: &str) -> PlatformResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM volume WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

fn decode(row: &wabot::sqlite::rusqlite::Row<'_>) -> wabot::sqlite::rusqlite::Result<Volume> {
    Ok(Volume {
        id: row.get(0)?,
        service_id: row.get(1)?,
        name: row.get(2)?,
        path: row.get(3)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{projects, replicas, services};

    async fn service() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "demo").await.expect("project");
        let service = services::create(&database, &project.id, "db", "postgres:17-alpine", &[])
            .await
            .expect("service");
        (database, service.id)
    }

    /// The rule the whole module rests on: two copies of one service
    /// get two directories. One shared between them would be two
    /// servers writing one data directory.
    #[test]
    fn every_copy_stores_its_data_somewhere_of_its_own() {
        let data = Path::new("/var/lib/wabot-deploy");
        let first = directory(data, "demo.db", "data");
        let second = directory(data, "demo.db.2", "data");

        assert_eq!(
            first,
            Path::new("/var/lib/wabot-deploy/volumes/demo.db/data")
        );
        assert_ne!(first, second);
    }

    /// Slot 1's container id has no suffix, so the directory a node has
    /// been using keeps its name when a second copy appears beside it.
    #[test]
    fn a_second_copy_does_not_rename_the_first_ones_directory() {
        let data = Path::new("/var/lib/wabot-deploy");
        let replica = |slot| replicas::Replica {
            id: "rp-1".into(),
            service_id: "svc-1".into(),
            node_id: None,
            slot,
            address: None,
            overlay_port: None,
            last_error: None,
            evicted_at: None,
            reserved_host: None,
        };

        assert_eq!(
            directory(data, &replica(1).container_id("demo", "db"), "data"),
            Path::new("/var/lib/wabot-deploy/volumes/demo.db/data")
        );
        assert_eq!(
            directory(data, &replica(2).container_id("demo", "db"), "data"),
            Path::new("/var/lib/wabot-deploy/volumes/demo.db.2/data")
        );
    }

    #[tokio::test]
    async fn a_volume_round_trips() {
        let (database, service) = service().await;
        create(&database, &service, "Data", "/var/lib/postgresql/data")
            .await
            .expect("created");

        let stored = of_service(&database, &service).await.expect("read");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "data");
        assert_eq!(stored[0].path, "/var/lib/postgresql/data");
    }

    /// Mounting over `/proc` is a container that does not start;
    /// mounting over `/etc/resolv.conf` is one that starts and resolves
    /// nothing, which looks like the image's fault.
    #[tokio::test]
    async fn the_mounts_a_container_needs_cannot_be_mounted_over() {
        let (database, service) = service().await;
        for reserved in ["/proc", "/dev/shm", "/etc/resolv.conf", "/", "/sys/"] {
            let error = create(&database, &service, "data", reserved)
                .await
                .expect_err(reserved);
            assert!(
                error.to_string().contains("needs to run"),
                "{reserved}: {error}"
            );
        }
    }

    #[tokio::test]
    async fn a_mount_point_is_an_absolute_path_without_traversal() {
        let (database, service) = service().await;
        for bad in ["data", "./data", "var/lib/data", "/var/../etc"] {
            assert!(
                create(&database, &service, "data", bad).await.is_err(),
                "{bad}"
            );
        }
    }

    /// `/data` and `/data/` are one destination, and two rows for it
    /// would be two mounts racing for it.
    #[tokio::test]
    async fn a_trailing_slash_is_the_same_mount_point() {
        let (database, service) = service().await;
        create(&database, &service, "first", "/data")
            .await
            .expect("created");

        let error = create(&database, &service, "second", "/data/")
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("already keeps data"), "{error}");
    }

    /// Deleting the service takes the rows. It does **not** take the
    /// bytes — that is `discard`, and its only caller is a confirmation
    /// somebody typed.
    #[tokio::test]
    async fn the_rows_go_with_the_service() {
        let (database, service) = service().await;
        create(&database, &service, "data", "/data")
            .await
            .expect("created");

        services::delete(&database, &service).await.expect("delete");
        assert!(of_service(&database, &service)
            .await
            .expect("read")
            .is_empty());
    }

    #[test]
    fn discarding_a_directory_that_is_not_there_is_already_done() {
        let dir = tempfile::tempdir().expect("tempdir");
        discard(dir.path(), "demo.db").expect("no such directory is not an error");
    }

    #[test]
    fn ensuring_is_convergent_and_keeps_what_is_there() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = ensure(dir.path(), "demo.db", "data").expect("made");
        std::fs::write(path.join("PG_VERSION"), "17").expect("write");

        let again = ensure(dir.path(), "demo.db", "data").expect("again");
        assert_eq!(path, again);
        assert!(
            again.join("PG_VERSION").exists(),
            "a redeploy wiped the data"
        );

        discard(dir.path(), "demo.db").expect("discard");
        assert!(!path.exists());
    }

    /// Reported, never removed: a directory whose rows are missing for
    /// a reason nobody has understood yet is one somebody can still
    /// recover from.
    #[test]
    fn a_directory_no_container_claims_is_reported() {
        let dir = tempfile::tempdir().expect("tempdir");
        ensure(dir.path(), "demo.db", "data").expect("made");
        ensure(dir.path(), "gone.db", "data").expect("made");

        let orphans = orphans(dir.path(), &["demo.db".to_string()]);
        assert_eq!(orphans.len(), 1);
        assert!(orphans[0].ends_with("gone.db"), "{orphans:?}");
        assert!(
            orphans[0].exists(),
            "reporting an orphan must not remove it"
        );
    }
}
