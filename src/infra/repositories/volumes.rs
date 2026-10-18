//! A [`Volume`]'s data access, in both stores it has: the rows, and the
//! directories on disk.

use std::path::{Path, PathBuf};

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::volume::Volume;
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::domain::values::mount_point::MountPoint;
use crate::infra::clock::now_ms;

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
/// How much this copy's volumes hold, or `None` when it has none.
///
/// Every volume of the copy at once: a service could declare two, and the
/// question somebody is asking is what this copy costs the disk.
///
/// `None` rather than nought for a copy with nothing on disk, because a
/// database showing 0 B is a reading somebody would act on and this is the
/// absence of a reading.
pub fn used_by(data_dir: &Path, container_id: &str) -> Option<u64> {
    let path = root(data_dir).join(container_id);
    if !path.exists() {
        return None;
    }
    Some(crate::infra::node::disk::used(&path).bytes)
}

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

pub struct Filled {
    pub entries: u64,
    /// What the archive tried to write outside the directory.
    pub skipped: Vec<String>,
}

/// Unpack an archive into one copy's directory for one volume, over what
/// is there: a file of the same name is replaced, nothing else is removed.
pub fn fill(
    data_dir: &Path,
    container_id: &str,
    name: &str,
    archive: &Path,
    owner: Option<(u32, u32)>,
) -> std::io::Result<Filled> {
    let into = ensure(data_dir, container_id, name)?;
    let file = std::fs::File::open(archive)?;
    let compressed = archive
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".gz") || name.ends_with(".tgz"));
    let reader: Box<dyn std::io::Read> = match compressed {
        true => Box::new(flate2::read::GzDecoder::new(file)),
        false => Box::new(file),
    };
    let mut unpacking = tar::Archive::new(reader);
    unpacking.set_preserve_permissions(true);
    unpacking.set_overwrite(true);
    unpacking.set_preserve_ownerships(owner.is_none() && running_as_root());

    let mut filled = Filled {
        entries: 0,
        skipped: Vec::new(),
    };
    for entry in unpacking.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.display().to_string();
        match entry.unpack_in(&into)? {
            true => filled.entries += 1,
            false => filled.skipped.push(path),
        }
    }
    if let Some((uid, gid)) = owner {
        own_tree(&into, uid, gid)?;
    }
    Ok(filled)
}

#[cfg(unix)]
fn running_as_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
fn running_as_root() -> bool {
    false
}

#[cfg(unix)]
fn own_tree(path: &Path, uid: u32, gid: u32) -> std::io::Result<()> {
    std::os::unix::fs::lchown(path, Some(uid), Some(gid))?;
    if std::fs::symlink_metadata(path)?.is_dir() {
        for entry in std::fs::read_dir(path)? {
            own_tree(&entry?.path(), uid, gid)?;
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn own_tree(_path: &Path, _uid: u32, _gid: u32) -> std::io::Result<()> {
    Ok(())
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
    let name = crate::domain::values::slug::Slug::of(name)
        .as_str()
        .to_string();
    if name.is_empty() || name.len() > 32 {
        return Err(PlatformError::Refused(
            "a volume name is 1 to 32 characters a directory can hold".into(),
        ));
    }
    let path = MountPoint::parse(path)?.to_string();

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

/// Every directory this service keeps data in.
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

    fn entry(archive: &mut tar::Builder<Vec<u8>>, raw_name: &str, body: &[u8]) {
        let mut header = tar::Header::new_old();
        header.as_old_mut().name[..raw_name.len()].copy_from_slice(raw_name.as_bytes());
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        archive.append(&header, body).expect("appended");
    }

    /// An archive is somebody else's file, unpacked as root. Built by hand,
    /// because the library refuses to write these two names itself.
    #[test]
    fn a_volume_is_filled_and_nothing_in_the_archive_leaves_it() {
        let node = tempfile::tempdir().expect("tempdir");
        let mut archive = tar::Builder::new(Vec::new());
        entry(&mut archive, "images/2020/07/cover.jpg", b"pixels");
        entry(&mut archive, "../escape.txt", b"out");
        entry(&mut archive, "/etc/abs.txt", b"out");
        let tarball = archive.into_inner().expect("tar");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tarball).expect("gz");
        let path = node.path().join("content.tar.gz");
        std::fs::write(&path, gz.finish().expect("gz")).expect("written");

        let filled = fill(node.path(), "shop.ghost", "content", &path, None).expect("filled");

        let volume = directory(node.path(), "shop.ghost", "content");
        assert_eq!(
            std::fs::read(volume.join("images/2020/07/cover.jpg")).expect("unpacked"),
            b"pixels"
        );
        assert!(!volume.parent().expect("parent").join("escape.txt").exists());
        assert_eq!(filled.skipped, vec!["../escape.txt".to_string()]);
        // An absolute name is read as relative to the volume, as GNU tar
        // reads it, so it lands inside rather than over the node's /etc.
        assert!(volume.join("etc/abs.txt").exists());
        assert_eq!(filled.entries, 2);
    }
    use crate::infra::repositories::services;

    async fn service() -> (SqliteDatabase, String) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &database,
            &project.id,
            "db",
            "docker.io/library/postgres:17-alpine",
            &[],
        )
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
        let replica = |slot| crate::domain::entities::replica::Replica {
            id: "rp-1".into(),
            service_id: "svc-1".into(),
            node_id: None,
            slot,
            address: None,
            overlay_port: None,
            last_error: None,
            evicted_at: None,
            reserved_host: None,
            memory_bytes: None,
            disk_bytes: None,
            cpu_millicores: None,
            following: None,
            wal_held: None,
            placed_by: None,
            placed_note: None,
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
