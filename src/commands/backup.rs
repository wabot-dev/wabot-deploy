//! `wabot-deploy backup` — everything this node would need to be itself
//! again.
//!
//! ## Three things, and only one of them is hard
//!
//! - **The node's own database.** `VACUUM INTO`, which is what the
//!   updater already uses: a byte copy of a live SQLite is a copy of a
//!   half-finished transaction.
//! - **Images.** Deliberately not copied. They are in a registry and
//!   come back with a pull, and a backup that carried them would be
//!   gigabytes of things somebody else is already keeping.
//! - **Volumes.** The hard one, and the reason this is a command rather
//!   than a `cp -a`.
//!
//! ## A file copy of a running database is not a backup
//!
//! It is a copy of a data directory mid-write, which restores into a
//! server that will not start — the same class of mistake as copying
//! SQLite by hand, one directory over. So a managed engine's volume is
//! copied **by the engine**: `pg_basebackup`, which is the same tool and
//! the same container pattern that seeds a standby, and which produces a
//! directory Postgres will open.
//!
//! A plain volume gets a file copy, and the manifest says so in those
//! words. Whether that is good enough is the operator's to decide and
//! not this node's to decide silently: a process that was mid-write is
//! a file that is mid-write, and only the person who knows what runs in
//! there knows whether that matters.
//!
//! ## A directory, not an archive
//!
//! No tar, no compression, no new dependency. What comes out is
//! something `rsync` and `scp -r` already understand, a half-finished
//! transfer is visible as a half-finished directory, and the one thing
//! an operator will actually do with it — move it off this machine — is
//! the thing every tool on the box can already do.
//!
//! ## And a backup on the same disk is not a backup
//!
//! This writes where it is told and says, every time, that leaving it
//! here protects against nothing. It cannot make somebody move it. It
//! can refuse to pretend.

use std::path::{Path, PathBuf};

use crate::config::Config;

/// What a backup directory holds, and how it was made.
///
/// Written as JSON beside the copies, and read by `restore`. The version
/// is first because the one thing this file must be able to say to a
/// newer binary is "you are older than me".
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    /// The format, not the product. It changes when the *shape* of a
    /// backup changes, which is rarer than a release.
    pub format: u32,
    /// What made it, for somebody reading a directory a year later.
    pub taken_by: String,
    pub taken_at: i64,
    /// **The node's identity.** Minted at `install`, kept for ever, and
    /// what every other node calls this one — so a restore that did not
    /// carry it would produce a *different node*, one that has to
    /// re-join while every other machine goes on holding rows about a
    /// ghost. `restore` asks which of the two is wanted; this is what
    /// makes the first possible.
    pub node_id: Option<String>,
    pub node_name: Option<String>,
    /// One entry per volume copied.
    pub volumes: Vec<Copied>,
}

/// One volume, and what its copy is worth.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Copied {
    /// The container id it belongs to, which is what names the
    /// directory on the node it is restored to.
    pub container: String,
    pub bytes: u64,
    /// How it was taken, in the words that decide whether it restores.
    pub how: How,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum How {
    /// Taken by the engine that owns it, with its own tool. Restores.
    Consistent,
    /// A file copy, taken while whatever writes it was running. It may
    /// restore, and it may restore into a process that will not start.
    /// The manifest says which so that nobody has to guess later.
    CrashConsistent,
}

/// The format this binary writes and reads.
pub const FORMAT: u32 = 1;

pub async fn run(config: Config, into: Option<PathBuf>) -> anyhow::Result<i32> {
    let database = crate::db::open(&config.database_path()).await?;

    let into = into.unwrap_or_else(|| {
        config
            .node
            .data_dir
            .join("backups")
            .join(format!("node-{}", crate::platform::now_ms()))
    });

    // Refused rather than merged. A backup written over another one is
    // two half-backups that look like one, and the shapes that produces
    // — an old volume beside a new database — are the ones nobody
    // notices until a restore.
    if into.exists() {
        println!("{} already exists; give an empty path", into.display());
        return Ok(1);
    }
    std::fs::create_dir_all(&into)?;

    let me = crate::network::me(&database).await.ok().flatten();
    println!("backing up to {}", into.display());

    // The database first. It carries the identity, the grants, the keys
    // and every row that describes what the volumes below are *for* — a
    // volume without it is bytes nothing can name.
    let db_copy = into.join("node.db");
    let destination = db_copy.to_string_lossy().to_string();
    database
        .write(move |connection| {
            connection.execute("VACUUM INTO ?1", [destination])?;
            Ok(())
        })
        .await?;
    println!("  database    {}", human(file_size(&db_copy)));

    let mut volumes = copy_volumes(&database, &config, &into).await;
    volumes.extend(back_up_engines(&database, &config, &into).await);
    let manifest = Manifest {
        format: FORMAT,
        taken_by: format!("wabot-deploy {}", crate::api::VERSION),
        taken_at: crate::platform::now_ms(),
        node_id: me.as_ref().map(|node| node.id.clone()),
        node_name: me.as_ref().map(|node| node.name.clone()),
        volumes,
    };
    std::fs::write(
        into.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    let engines = crate::platform::services::all(&database, None)
        .await
        .map(|services| {
            services
                .iter()
                .filter(|service| service.kind.is_managed())
                .count()
        })
        .unwrap_or(0);
    let taken = manifest
        .volumes
        .iter()
        .filter(|copied| copied.how == How::Consistent)
        .count();
    if taken < engines {
        println!();
        println!(
            "  {} of {engines} database(s) were NOT copied — see above.",
            engines - taken
        );
        println!("  This backup does not hold them.");
    }

    let crash: Vec<&Copied> = manifest
        .volumes
        .iter()
        .filter(|copied| copied.how == How::CrashConsistent)
        .collect();
    if !crash.is_empty() {
        println!();
        println!("  {} volume(s) were copied while running:", crash.len());
        for copied in &crash {
            println!("    {}", copied.container);
        }
        println!("  Those are crash-consistent — the same as if the machine had lost");
        println!("  power. Whether that restores is a question about what runs in them.");
    }

    database.close().await?;
    println!();
    println!("  Move it off this machine. A backup on the same disk protects");
    println!("  against nothing that has ever happened to a disk.");
    Ok(0)
}

/// Which backup a restore to this moment has to start from.
///
/// **The newest one taken at or before the target.** Not the newest of
/// all, which is the mistake that is easy to make and impossible to
/// recover from: replaying forwards is the only direction there is, so a
/// backup taken *after* the moment somebody wants already contains what
/// they are trying to undo. It cannot be rewound to reach them.
///
/// `None` when every backup is newer than the target — the moment is
/// before anything this node kept, and saying so is the only honest
/// answer. An operator who asked for last Tuesday and got Thursday's
/// data with no warning would find out by reading rows that should not
/// exist.
pub fn base_for<'a>(
    taken: &'a [(PathBuf, Manifest)],
    target: i64,
) -> Option<&'a (PathBuf, Manifest)> {
    // `taken` is newest first, so the first one at or before the target
    // is the newest such — no scan of the rest.
    taken
        .iter()
        .find(|(_, manifest)| manifest.taken_at <= target)
}

/// What a database can be restored to, and what it cannot.
///
/// **Four answers, and three of them are "not what you think".** The
/// question an operator is really asking is "how far back can I go, and
/// how recent can I land", and every way of getting that wrong is a
/// promise that fails at the moment it is called on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Window {
    /// Nothing is kept. Restoring means the last backup and no further.
    NotKeeping,
    /// The log is being kept and **there is no backup to anchor it**.
    /// Disk is being spent on something nothing can use: a base backup
    /// is what the log is replayed *onto*, and without one the whole
    /// archive is unreachable. The worst of the four, because it is the
    /// one that looks like it is working.
    NoAnchor,
    /// A backup exists and no log has arrived yet — a database that has
    /// just been turned on, or one nobody has written to.
    OnlyTheBackup { at: i64 },
    /// Any moment between the two. `to` is the time of the last
    /// **usable** segment, which is not the newest one when the run is
    /// broken: recovery stops at a hole, so a gap makes the segment
    /// before it the real end.
    Between {
        from: i64,
        to: i64,
        /// Set when the run is broken, so the page can say that
        /// everything after it is being kept and cannot be reached.
        gap: bool,
    },
}

/// Work out the window for one database's archive.
///
/// `anchor` is when the oldest kept base backup was taken, which is the
/// earliest moment any restore could target — everything before it is
/// gone whatever the archive holds.
pub fn window(data_dir: &Path, container: &str, anchor: Option<i64>, keeping: bool) -> Window {
    if !keeping {
        return Window::NotKeeping;
    }
    let archive = crate::deploy::database::archive_dir(data_dir, container);
    let held = crate::platform::wal::held(&archive);

    let Some(from) = anchor else {
        // Keeping log with nothing to replay it onto. Said as its own
        // answer rather than folded into "nothing yet", because the two
        // want opposite things done about them: one waits, and this one
        // needs a backup taking now.
        return match held.segments {
            0 => Window::NotKeeping,
            _ => Window::NoAnchor,
        };
    };

    // The last segment that can actually be reached. A gap ends the
    // window there whatever arrived afterwards.
    let last = match &held.gap {
        Some(missing) => newest_before(&archive, missing),
        None => held
            .newest
            .as_ref()
            .and_then(|name| modified(&archive, name)),
    };

    match last {
        Some(to) if to > from => Window::Between {
            from,
            to,
            gap: held.gap.is_some(),
        },
        // A segment older than the backup reaches nothing: the backup
        // already contains that moment.
        _ => Window::OnlyTheBackup { at: from },
    }
}

/// When the newest segment before this one was archived.
fn newest_before(archive: &Path, missing: &str) -> Option<i64> {
    std::fs::read_dir(archive)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let stem = name.strip_suffix(".gz").unwrap_or(&name).to_string();
            (stem.len() == 24 && stem.as_str() < missing).then(|| at(&entry))?
        })
        .max()
}

fn modified(archive: &Path, name: &str) -> Option<i64> {
    std::fs::read_dir(archive)
        .ok()?
        .flatten()
        .find_map(|entry| {
            let found = entry.file_name().to_string_lossy().to_string();
            found.starts_with(name).then(|| at(&entry))?
        })
}

fn at(entry: &std::fs::DirEntry) -> Option<i64> {
    let modified = entry.metadata().ok()?.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(since.as_millis() as i64)
}

/// Drop what has expired, and the log no survivor needs.
///
/// **In that order, and it matters.** Deleting log first would leave a
/// window with a hole in it for as long as the pass took; deleting
/// backups first means the log is measured against what actually
/// survives. A pass interrupted between the two leaves an archive that
/// is larger than it needs to be, which is the harmless direction.
///
/// Returns what it freed, so the caller can say something on the passes
/// where it acted and nothing on the many where it did not.
pub fn sweep(data_dir: &Path, now: i64) -> (usize, u64) {
    let backups = taken(data_dir);
    let (keep, drop) = keeping(&backups, now);

    let mut freed = 0;
    let mut removed = 0;
    for path in drop {
        let size = crate::node::disk::used(path).bytes;
        match std::fs::remove_dir_all(path) {
            Ok(()) => {
                tracing::info!(backup = %path.display(), "expired backup removed");
                removed += 1;
                freed += size;
            }
            Err(error) => tracing::warn!(backup = %path.display(), %error, "could not remove it"),
        }
    }

    // What the oldest surviving backup needs, per database. Each
    // archive is asked about its own: two databases have two timelines
    // and two positions, and one number for both would be right for one
    // of them by accident.
    let oldest = keep.last().and_then(|path| {
        std::fs::read_dir(path.join("volumes")).ok().map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .collect::<Vec<_>>()
        })
    });
    let Some(volumes) = oldest else {
        return (removed, freed);
    };

    for volume in volumes {
        let Some(container) = volume
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
        else {
            continue;
        };
        // No manifest is "needs everything" — the one answer that
        // cannot put a hole in a window. `prune` takes `None` and does
        // nothing with it.
        let needed = std::fs::read_to_string(volume.join("backup_manifest"))
            .ok()
            .and_then(|manifest| crate::platform::wal::start_of(&manifest))
            .map(|(timeline, lsn)| crate::platform::wal::segment_name(timeline, lsn));

        let archive = crate::deploy::database::archive_dir(data_dir, &container);
        let pruned = crate::platform::wal::prune(&archive, needed.as_deref());
        if pruned.removed > 0 {
            tracing::info!(
                database = %container, removed = pruned.removed, freed = pruned.freed,
                kept = pruned.kept, "archived log pruned"
            );
            removed += pruned.removed;
            freed += pruned.freed;
        }
    }
    (removed, freed)
}

/// How long a database can be restored to any moment within.
///
/// Seven days, and it is the number an operator is really choosing when
/// they turn archiving on: everything inside it costs disk, and
/// everything outside it is gone. Not configurable yet — a default that
/// somebody has to think about is better than a field they have to fill
/// in before the feature works at all, and this is the value at which
/// "somebody dropped a table on Friday and noticed on Monday" is
/// recoverable.
pub const KEEP_DAYS: i64 = 7;

/// Every backup this node has taken, newest first.
///
/// Read off the disk rather than a table, deliberately. A backup is a
/// directory somebody can move, copy or delete with the tools they
/// already have, and a row claiming one exists where the directory does
/// not is worse than no row — this way the disk is the record and
/// cannot disagree with itself.
pub fn taken(data_dir: &Path) -> Vec<(PathBuf, Manifest)> {
    let mut found: Vec<(PathBuf, Manifest)> = std::fs::read_dir(data_dir.join("backups"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let manifest = std::fs::read_to_string(entry.path().join("manifest.json")).ok()?;
            Some((entry.path(), serde_json::from_str(&manifest).ok()?))
        })
        .collect();
    found.sort_by_key(|(_, manifest): &(PathBuf, Manifest)| -manifest.taken_at);
    found
}

/// Which backups to keep, and which one anchors the window.
///
/// **The newest one older than the window, and everything after it.**
/// Not "everything inside the window", which is the version that reads
/// right and is wrong: on the morning after a seven-day-old backup
/// expires, the newest remaining one might be six days old, and every
/// moment before it would be unrecoverable — the window would silently
/// shrink to six days and then to five.
///
/// So the anchor is the *last one that predates the window*, which is
/// what makes the whole window reachable, and it is only dropped once a
/// newer backup has taken over that job.
pub fn keeping(taken: &[(PathBuf, Manifest)], now: i64) -> (Vec<&PathBuf>, Vec<&PathBuf>) {
    let horizon = now - KEEP_DAYS * 24 * 60 * 60 * 1000;
    let mut keep = Vec::new();
    let mut drop = Vec::new();
    let mut anchored = false;

    // Newest first, so the first one older than the horizon is the
    // anchor and everything past it is expired.
    for (path, manifest) in taken {
        match manifest.taken_at >= horizon || !anchored {
            true => {
                if manifest.taken_at < horizon {
                    anchored = true;
                }
                keep.push(path);
            }
            false => drop.push(path),
        }
    }
    (keep, drop)
}

/// Copy every volume, each the way its owner allows.
async fn copy_volumes(
    database: &wabot::sqlite::SqliteDatabase,
    config: &Config,
    into: &Path,
) -> Vec<Copied> {
    let root = crate::platform::volumes::root(&config.node.data_dir);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };

    // Which container ids belong to a managed engine, so the ones that
    // need their own tool can be told from the ones that do not. Read
    // from the rows rather than guessed from the directory name: the
    // name carries a project and a service and says nothing about what
    // runs inside.
    let managed = managed_containers(database).await;
    let mut copied = Vec::new();

    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let container = entry.file_name().to_string_lossy().to_string();
        let destination = into.join("volumes").join(&container);

        // A managed engine's data directory is copied by the engine —
        // see the module docs — and that happens below, per service,
        // because it needs the rows to know which copy to read from.
        if managed.contains(&container) {
            continue;
        }

        match copy_tree(&entry.path(), &destination) {
            Ok(bytes) => {
                println!("  volume      {container}  {}", human(bytes));
                copied.push(Copied {
                    container,
                    bytes,
                    how: How::CrashConsistent,
                });
            }
            Err(error) => println!("  volume      {container} — could not copy: {error}"),
        }
    }
    copied
}

/// Let every managed engine here copy itself.
///
/// One `pg_basebackup` per database, taken from a read-only copy when
/// there is one — see `Deployer::back_up_engine`. This is the half a
/// file copy cannot do, and the reason the whole thing is a command.
///
/// A database that could not be copied is **reported and skipped**, and
/// the manifest simply has no entry for it. That is deliberate: a
/// backup missing a database is a backup somebody can still use for the
/// rest, and an error that stopped the run would throw away the copies
/// that did work.
async fn back_up_engines(
    database: &wabot::sqlite::SqliteDatabase,
    config: &Config,
    into: &Path,
) -> Vec<Copied> {
    let (Ok(services), Ok(projects)) = (
        crate::platform::services::all(database, None).await,
        crate::platform::projects::all(database).await,
    ) else {
        return Vec::new();
    };
    let deployer = crate::deploy::Deployer::new(std::sync::Arc::new(database.clone()), config);
    let mut copied = Vec::new();

    for service in services.iter().filter(|service| service.kind.is_managed()) {
        let Some(project) = projects
            .iter()
            .find(|project| project.id == service.project_id)
        else {
            continue;
        };
        let container = format!("{}.{}", project.slug, service.slug);
        let destination = into.join("volumes").join(&container);

        match deployer
            .back_up_engine(project, service, &destination)
            .await
        {
            Ok(bytes) => {
                println!("  database    {container}  {}", human(bytes));
                copied.push(Copied {
                    container,
                    bytes,
                    // The engine's own tool wrote it, which is what
                    // makes this the one kind of copy that restores.
                    how: How::Consistent,
                });
            }
            Err(reason) => {
                println!("  database    {container} — NOT COPIED: {reason}");
                // Left on disk rather than removed: a half-written
                // backup that somebody can look at beats one this
                // command tidied away.
            }
        }
    }
    copied
}

/// The container ids of every copy of a managed engine on this node.
async fn managed_containers(database: &wabot::sqlite::SqliteDatabase) -> Vec<String> {
    let (Ok(services), Ok(projects), Ok(mine)) = (
        crate::platform::services::all(database, None).await,
        crate::platform::projects::all(database).await,
        crate::platform::replicas::here(database).await,
    ) else {
        return Vec::new();
    };

    mine.iter()
        .filter_map(|replica| {
            let service = services
                .iter()
                .find(|service| service.id == replica.service_id)?;
            if !service.kind.is_managed() {
                return None;
            }
            let project = projects
                .iter()
                .find(|project| project.id == service.project_id)?;
            Some(replica.container_id(&project.slug, &service.slug))
        })
        .collect()
}

/// Copy a directory, and say how much it holds.
///
/// Symlinks are not followed, for the reason `disk::used` gives: a
/// volume that pointed at `/` would otherwise copy the machine.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(to)?;
    let mut bytes = 0;

    for entry in std::fs::read_dir(from)?.flatten() {
        let metadata = entry.metadata()?;
        let target = to.join(entry.file_name());
        if metadata.is_dir() {
            bytes += copy_tree(&entry.path(), &target)?;
        } else if metadata.is_file() {
            bytes += std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(bytes)
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn human(bytes: u64) -> String {
    crate::node::memory::human(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The newest backup at or before the moment, never the newest of
    /// all.** Replaying only goes forwards, so a backup taken after the
    /// moment somebody wants already contains the thing they are trying
    /// to undo — and no amount of log replay walks back to them.
    ///
    /// The failure this prevents is the quiet one: an operator asks for
    /// last Tuesday, gets Thursday's data, and finds out by reading rows
    /// that should not exist.
    #[test]
    fn a_restore_starts_from_the_backup_before_the_moment() {
        let day = 24 * 60 * 60 * 1000i64;
        let at = |taken_at: i64| {
            (
                PathBuf::from(format!("backup-{taken_at}")),
                Manifest {
                    format: FORMAT,
                    taken_by: "test".into(),
                    taken_at,
                    node_id: None,
                    node_name: None,
                    volumes: Vec::new(),
                },
            )
        };
        // Newest first, as `taken` returns them.
        let backups = vec![at(10 * day), at(5 * day), at(1 * day)];

        // A moment between two backups starts from the earlier one.
        let picked = base_for(&backups, 7 * day).expect("one before it");
        assert_eq!(picked.1.taken_at, 5 * day);

        // A moment after everything starts from the newest.
        assert_eq!(
            base_for(&backups, 20 * day).expect("the newest").1.taken_at,
            10 * day
        );

        // Exactly at a backup is that backup: it holds that moment.
        assert_eq!(
            base_for(&backups, 5 * day).expect("that one").1.taken_at,
            5 * day
        );

        // And a moment before anything kept has no answer, rather than
        // silently getting one that is too late.
        assert!(base_for(&backups, 12 * 60 * 60 * 1000).is_none());
        assert!(base_for(&[], 5 * day).is_none());
    }

    /// The four answers, and the one that matters most is `NoAnchor`.
    ///
    /// Keeping log with no backup to replay it onto is the state that
    /// *looks* like it is working — the archive fills, the disk goes
    /// down, everything reports normally — and recovers nothing at all.
    /// A base backup is what the log is replayed onto; without one the
    /// whole archive is unreachable.
    #[test]
    fn a_window_says_which_of_the_four_states_it_is_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        let archive = crate::deploy::database::archive_dir(dir.path(), "demo.orders");
        std::fs::create_dir_all(&archive).expect("mkdir");
        let write = |name: &str| {
            std::fs::write(archive.join(name), vec![0u8; 16]).expect("write");
        };

        // Off is off, whatever is on the disk.
        assert_eq!(
            window(dir.path(), "demo.orders", Some(1_000), false),
            Window::NotKeeping
        );

        // On, and nothing has arrived: waiting, not broken.
        assert_eq!(
            window(dir.path(), "demo.orders", None, true),
            Window::NotKeeping
        );

        // On, log arriving, and no backup — the dangerous one.
        write("000000010000000000000010.gz");
        assert_eq!(
            window(dir.path(), "demo.orders", None, true),
            Window::NoAnchor,
            "log with nothing to replay it onto recovers nothing"
        );

        // A backup, and log older than it: the backup already holds
        // those moments, so there is nothing further to reach.
        let anchored = window(dir.path(), "demo.orders", Some(i64::MAX - 1), true);
        assert!(
            matches!(anchored, Window::OnlyTheBackup { .. }),
            "{anchored:?}"
        );

        // A backup, and log after it: a real window.
        let between = window(dir.path(), "demo.orders", Some(0), true);
        match between {
            Window::Between { from, to, gap } => {
                assert_eq!(from, 0);
                assert!(to > 0);
                assert!(!gap);
            }
            other => panic!("expected a window: {other:?}"),
        }

        // And a hole ends it early, whatever arrived afterwards.
        write("000000010000000000000011.gz");
        write("000000010000000000000099.gz");
        let broken = window(dir.path(), "demo.orders", Some(0), true);
        assert!(
            matches!(broken, Window::Between { gap: true, .. }),
            "a gap has to reach the page: {broken:?}"
        );
    }

    /// The window is anchored by the newest backup *older* than it, and
    /// that is the part which reads wrong and is right.
    ///
    /// Keeping "everything inside the window" is the obvious rule and it
    /// shrinks: the morning after the seven-day-old backup expires, the
    /// newest one left might be six days old, and every moment before it
    /// becomes unrecoverable. The window would quietly become six days,
    /// then five. So the one that predates the window stays until a
    /// newer one can take over its job.
    #[test]
    fn the_window_is_anchored_by_the_backup_before_it() {
        let day = 24 * 60 * 60 * 1000i64;
        let now = 100 * day;
        let at = |days: i64| {
            (
                PathBuf::from(format!("backup-{days}")),
                Manifest {
                    format: FORMAT,
                    taken_by: "test".into(),
                    taken_at: now - days * day,
                    node_id: None,
                    node_name: None,
                    volumes: Vec::new(),
                },
            )
        };

        // Newest first, as `taken` returns them.
        let backups = vec![at(1), at(6), at(9), at(20)];
        let (keep, drop) = keeping(&backups, now);

        assert_eq!(keep.len(), 3, "and the nine-day-old one anchors the week");
        assert!(keep.iter().any(|path| path.ends_with("backup-9")));
        assert_eq!(drop.len(), 1);
        assert!(drop[0].ends_with("backup-20"));

        // With everything inside the window, nothing anchors it from
        // outside and nothing is dropped: there is no older one to
        // spare.
        let recent = vec![at(1), at(2)];
        let (keep, drop) = keeping(&recent, now);
        assert_eq!(keep.len(), 2);
        assert!(drop.is_empty());

        // And one backup is never dropped, however old. It is the only
        // thing that could be restored.
        let only = vec![at(400)];
        let (keep, drop) = keeping(&only, now);
        assert_eq!(keep.len(), 1);
        assert!(drop.is_empty(), "the last backup is not rubbish");
    }

    /// A copy is worth what its manifest says it is worth, and the two
    /// answers are not interchangeable: one restores, and the other is
    /// the same as the machine having lost power. An operator reading a
    /// directory a year later has nothing else to go on.
    #[test]
    fn a_manifest_says_how_each_copy_was_taken() {
        let manifest = Manifest {
            format: FORMAT,
            taken_by: "wabot-deploy 0.10.0".into(),
            taken_at: 1_700_000_000_000,
            node_id: Some("nd-abc".into()),
            node_name: Some("box.example".into()),
            volumes: vec![Copied {
                container: "demo.api".into(),
                bytes: 4096,
                how: How::CrashConsistent,
            }],
        };

        let json = serde_json::to_string(&manifest).expect("serialise");
        let read: Manifest = serde_json::from_str(&json).expect("read back");
        assert_eq!(read.format, FORMAT);
        assert_eq!(read.volumes[0].how, How::CrashConsistent);
        // The identity, which is what makes a restore able to be *this*
        // node rather than a new one that has to re-join.
        assert_eq!(read.node_id.as_deref(), Some("nd-abc"));
    }

    /// The copy walks a tree and does not follow a symlink out of it.
    #[test]
    fn a_copy_holds_the_tree_and_nothing_it_points_at() {
        let from = tempfile::tempdir().expect("tempdir");
        let to = tempfile::tempdir().expect("tempdir");
        std::fs::write(from.path().join("one"), vec![0u8; 100]).expect("write");
        std::fs::create_dir(from.path().join("nested")).expect("mkdir");
        std::fs::write(from.path().join("nested").join("two"), vec![0u8; 50]).expect("write");

        let bytes = copy_tree(from.path(), &to.path().join("out")).expect("copied");
        assert_eq!(bytes, 150);
        assert!(to.path().join("out").join("nested").join("two").exists());
    }
}
