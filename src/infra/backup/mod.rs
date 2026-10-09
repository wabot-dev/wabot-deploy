//! `wabot-deploy backup` — everything this node would need to be itself
//! again.
//!
//! ## Three things, and only one of them is hard
//!
//! - **The node's own database.** `VACUUM INTO`, which is what the
//!   updater already uses: a byte copy of a live SQLite is a copy of a
//!   half-finished transaction.
//! - **Images this node is the only copy of.** Its own builds, which no
//!   registry anywhere else is holding. A public base image is not
//!   copied: it comes back with a pull, and carrying it would be
//!   gigabytes of something somebody else already keeps.
//! - **Volumes.** The hard one, and the reason this is a command rather
//!   than a `cp -a`.
//!
//! ## And only what something claims
//!
//! A volume directory outlives the copy that made it — nothing deletes
//! one, on purpose, because it is data somebody may still want. So the
//! disk holds directories for copies that have been moved off this node
//! or thrown off it, and those are **not** in a backup: there is no row
//! to restore them under, so they would be weight in every copy for ever
//! with nothing behind it. `backup` names each one it skipped, because
//! "my backup has everything on the disk" is the assumption, and a
//! backup is the worst place to be quietly wrong.
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
//! ## One root for the whole network
//!
//! Every node points at the same backup root, which is what makes an
//! S3 bucket or one directory on one machine the answer for a network
//! rather than for a node. Two things follow, and the second is why it
//! is worth doing at all:
//!
//! - **What is a node's own goes under its own id.** Two nodes writing
//!   `manifest.json` at the top of a shared root would be one node's
//!   backup, alternately.
//! - **Image layers go once, under their digest.** Two nodes running
//!   the same base image hold the same bytes; a backup that copied them
//!   per node would store the network's images as many times as there
//!   are machines. See `backup::blobs`, where the name being the hash
//!   is the whole of the deduplication.
//!
//! ```text
//! <root>/
//!   blobs/sha256/<hex>              every node, once
//!   nodes/<node id>/<taken at>/
//!     manifest.json
//!     node.db
//!     volumes/<container>/
//! ```
//!
//! The local default under `data_dir/backups` has the same shape, so
//! sending it somewhere shared is a copy rather than a conversion.
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

pub mod blobs;
pub mod destination;
pub mod plan;
pub mod restore_database_into;
pub mod s3;

use crate::infra::backup::destination::Destination;
use crate::infra::config::Config;
use std::path::{Path, PathBuf};

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
    /// The images this node's services need, which nowhere else has.
    #[serde(default)]
    pub images: Vec<Kept>,
}

/// One image, and the blobs it is made of.
///
/// The digests are in the shared store — see `backup::blobs` — and
/// this is the list that says which of them belong to this reference.
/// Without it a restore would have the bytes and no way to know which
/// image they add up to.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Kept {
    /// The reference as containerd knows it, which is what a service
    /// asks for.
    pub reference: String,
    /// The image's own top descriptor: what the manifest list or
    /// manifest is, so a restore can recreate the image record.
    pub digest: String,
    pub media_type: String,
    /// The top descriptor's size, which containerd wants when the image
    /// record goes back in.
    #[serde(default)]
    pub size: i64,
    /// Every blob, with its size. The size is recorded because
    /// containerd wants it when the blob goes back in.
    pub blobs: Vec<(String, i64)>,
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

/// One backup, with the caller's database handle.
///
/// Returns what it holds, or why there is nothing — both as a sentence
/// somebody reads: on a terminal it is beneath the narration below, and
/// on a schedule it is what lands on the row the console's Backup tab
/// reads. Everything else it has to say it says as it goes, which in the
/// serving process is the journal, because `println!` and `tracing` are
/// the same stream there.
pub async fn once(
    config: &Config,
    database: &wabot::sqlite::SqliteDatabase,
    into: Option<String>,
) -> Result<String, String> {
    // Read before anything is built, so a destination somebody mistyped
    // costs nothing. A `pg_basebackup` of every database first and *then*
    // "that is not a scheme I know" is a lot of work to throw away.
    let destination = match into.as_deref().map(Destination::parse) {
        Some(Ok(destination)) => destination,
        Some(Err(reason)) => {
            println!("{reason}");
            return Err(reason);
        }
        None => Destination::Local(config.node.data_dir.join("backups")),
    };

    // And the same for what the send will need. `rsync` being absent is
    // true before the backup starts and still true twenty minutes later,
    // so learning it at the moment of use means every database copied
    // for nothing.
    if let Err(reason) = destination.preflight() {
        println!("{reason}");
        return Err(reason);
    }

    // Where this node's own things go under whichever root was named —
    // see the module docs. A node with no id of its own has never been
    // on a network and cannot collide with anybody, so it gets a name
    // that says so rather than a blank path segment.
    let me = crate::infra::network::me(database).await.ok().flatten();
    // A remote destination is built here first and sent at the end — see
    // `destination`'s module docs for why it cannot be streamed. The
    // staging root sits inside `data_dir` so that it is the same
    // filesystem the volumes are on, which is what makes the copies a
    // rename's worth of work rather than a second full write.
    let root = match &destination {
        Destination::Local(path) => path.clone(),
        _ => config
            .node
            .data_dir
            .join("backups")
            .join(format!(".sending-{}", crate::infra::clock::now_ms())),
    };
    let into = root
        .join("nodes")
        .join(
            me.as_ref()
                .map(|node| node.id.clone())
                .unwrap_or_else(|| "this-node".into()),
        )
        .join(crate::infra::clock::now_ms().to_string());

    // Refused rather than merged. A backup written over another one is
    // two half-backups that look like one, and the shapes that produces
    // — an old volume beside a new database — are the ones nobody
    // notices until a restore.
    if into.exists() {
        let reason = format!("{} already exists; give an empty path", into.display());
        println!("{reason}");
        return Err(reason);
    }
    std::fs::create_dir_all(&into).map_err(|error| format!("{}: {error}", into.display()))?;

    match destination.is_remote() {
        true => println!("backing up to {}", destination.display()),
        false => println!("backing up to {}", into.display()),
    }

    // The database first. It carries the identity, the grants, the keys
    // and every row that describes what the volumes below are *for* — a
    // volume without it is bytes nothing can name.
    let db_copy = into.join("node.db");
    let db_copy_path = db_copy.to_string_lossy().to_string();
    database
        .write(move |connection| {
            connection.execute("VACUUM INTO ?1", [db_copy_path])?;
            Ok(())
        })
        .await
        .map_err(|error| format!("the node's own database could not be copied: {error}"))?;
    println!("  database    {}", human(file_size(&db_copy)));

    let mut volumes = copy_volumes(database, config, &into).await;
    let engines = back_up_engines(database, config, &into).await;
    // Moved out of the struct rather than cloned; `missed` below is what
    // the rest of this function needs from it.
    volumes.extend(engines.copied);
    // Into the *root*, not this backup's directory: blobs are the part
    // every node shares, and one node's copy of a layer is every node's.
    let images = keep_images(database, &root).await;
    let manifest = Manifest {
        format: FORMAT,
        taken_by: format!("wabot-deploy {}", crate::infra::update::VERSION),
        taken_at: crate::infra::clock::now_ms(),
        node_id: me.as_ref().map(|node| node.id.clone()),
        node_name: me.as_ref().map(|node| node.name.clone()),
        volumes,
        images,
    };
    let written = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("the manifest could not be written: {error}"))
        .and_then(|bytes| {
            std::fs::write(into.join("manifest.json"), bytes)
                .map_err(|error| format!("the manifest could not be written: {error}"))
        });
    // Refused rather than warned about: a directory with no
    // `manifest.json` in it is not a backup — `restore` says so in those
    // words — so a run that could not write one has produced nothing,
    // whatever else is on the disk.
    written?;

    if !engines.missed.is_empty() {
        println!();
        println!(
            "  {} database(s) here were NOT copied — see above.",
            engines.missed.len()
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

    // What it holds, in the units the page beside it uses. The database
    // and the volumes, which is everything under this run's own
    // directory — the blobs are the network's and are counted by nobody's
    // backup, on purpose, because they are one copy for every node.
    let held = format!(
        "{} in {} volume(s)",
        human(
            file_size(&db_copy)
                + manifest
                    .volumes
                    .iter()
                    .map(|copied| copied.bytes)
                    .sum::<u64>()
        ),
        manifest.volumes.len()
    );

    let delivered = match destination.is_remote() {
        false => {
            println!();
            println!("  Move it off this machine. A backup on the same disk protects");
            println!("  against nothing that has ever happened to a disk.");
            Ok(format!("{held} in {}", into.display()))
        }
        true => remote_copy(&root, &into, &destination, config, &held).await,
    };

    // **A backup that does not hold a database this node runs did not
    // work**, whatever the transfer said. The bytes are real and are
    // named above; what makes this a failure is what it *lacks* — and the
    // whole point of a schedule is that nobody is reading the narration,
    // so the shortfall has to be the value that comes back. Found on a
    // node: a database whose `pg_hba.conf` refused the base backup,
    // reported as a run that worked and holding nothing.
    match delivered {
        Ok(note) if !engines.missed.is_empty() => Err(format!(
            "{note} — but it does not hold {}",
            engines.missed.join("; ")
        )),
        other => other,
    }
}

/// Send it, and say where the bytes are either way.
async fn remote_copy(
    root: &Path,
    into: &Path,
    destination: &Destination,
    config: &Config,
    held: &str,
) -> Result<String, String> {
    println!();
    match crate::infra::backup::destination::send(root, destination, config).await {
        Ok(what) => {
            println!("  sent        {what} → {}", destination.display());
            // Only now. The staging copy is this backup's only existence
            // until the transfer says otherwise, and removing it before
            // that would be a command that reports success and holds
            // nothing.
            match std::fs::remove_dir_all(root) {
                Ok(()) => {}
                Err(error) => println!("  local copy  {} left behind: {error}", root.display()),
            }
            Ok(format!("{held} sent to {}", destination.display()))
        }
        Err(reason) => {
            println!("  NOT SENT    {reason}");
            println!();
            // Kept, and named. The bytes exist and the operator is the
            // only one who can decide what to do about a destination
            // that refused them — retrying by hand is a one-line rsync
            // from here.
            println!("  The backup is complete and still on this machine:");
            println!("    {}", into.display());
            // The reason, and where the bytes are. A row saying only
            // "could not send it" would leave whoever reads it looking
            // for a backup that is right there.
            Err(format!(
                "not sent: {reason}. It is still in {}",
                into.display()
            ))
        }
    }
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
pub fn base_for<'a, T>(
    taken: &'a [(T, Manifest)],
    target: i64,
    // The database it has to hold, or `None` for a whole-node restore,
    // where every backup holds what it holds.
    container: Option<&str>,
) -> Option<&'a (T, Manifest)> {
    // `taken` is newest first, so the first one at or before the target
    // is the newest such — no scan of the rest.
    //
    // **The manifest answers "does it hold this database".** Not the
    // disk: the same question has to be answerable of a bucket, and
    // downloading a backup to find out whether it is the one is the cost
    // this whole path exists to avoid. `back_up_engines` writes an entry
    // per volume it copied, so a database it could not copy is absent
    // from the list.
    taken
        .iter()
        .filter(|(_, manifest)| manifest.taken_at <= target)
        .find(|(_, manifest)| match container {
            Some(container) => holds(manifest, container),
            None => true,
        })
}

/// Which refusal fits: nothing old enough, or nothing holding it.
///
/// The two read very differently in the middle of a recovery. "No backup
/// was taken before that moment" means go back further; "none of them
/// holds this database" means the backups are fine and this database was
/// not in them — which is the shape a `pg_hba.conf` that refused the base
/// backup produced for five days on a real node.
fn refusal<T>(candidates: &[(T, Manifest)], at: i64, looked: PathBuf) -> NotRestored {
    match candidates
        .iter()
        .any(|(_, manifest)| manifest.taken_at <= at)
    {
        true => NotRestored::HoldsNoCopy(looked),
        false => NotRestored::NothingOldEnough(looked),
    }
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
    let archive = crate::infra::deploy::database::archive_dir(data_dir, container);
    let held = crate::infra::repositories::wal_archive::held(&archive);

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
pub fn sweep(data_dir: &Path, now: i64, keep_days: i64) -> (usize, u64) {
    let backups = taken(data_dir);
    let (keep, drop) = keeping(&backups, now, keep_days);

    let mut freed = 0;
    let mut removed = 0;
    for path in drop {
        let size = crate::infra::node::disk::used(path).bytes;
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
            .and_then(|manifest| crate::domain::policies::wal::start_of(&manifest))
            .map(|(timeline, lsn)| crate::domain::policies::wal::segment_name(timeline, lsn));

        let archive = crate::infra::deploy::database::archive_dir(data_dir, &container);
        let pruned = crate::infra::repositories::wal_archive::prune(&archive, needed.as_deref());
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
/// everything outside it is gone. This is the value at which "somebody
/// dropped a table on Friday and noticed on Monday" is recoverable.
///
/// ~~Not configurable yet.~~ It is a field now, on the node's Backup tab
/// — see [`crate::infra::backup::plan`] — and this is what a node that has
/// never said anything gets. The default stays here rather than moving
/// there because it is the number this function's own reasoning is
/// about, and because `sweep` has to be answerable against a directory
/// with no database anywhere near it.
pub const KEEP_DAYS: i64 = 7;

/// Every backup this node has taken, newest first.
///
/// Read off the disk rather than a table, deliberately. A backup is a
/// directory somebody can move, copy or delete with the tools they
/// already have, and a row claiming one exists where the directory does
/// not is worse than no row — this way the disk is the record and
/// cannot disagree with itself.
pub fn taken(data_dir: &Path) -> Vec<(PathBuf, Manifest)> {
    taken_in(&data_dir.join("backups"))
}

/// The same, for a root that is not this node's own.
///
/// A destination is a root like any other — one directory on one machine,
/// or a bucket — and `restore` has to be able to read one it did not
/// write. Split out when the console learned to restore from wherever the
/// backups actually are, which for a node with a schedule is *not* here:
/// the local staging copy is removed the moment the transfer succeeds.
pub fn taken_in(root: &Path) -> Vec<(PathBuf, Manifest)> {
    // Every node's, not only this one's. A shared root holds several,
    // and the manifest says whose each is — reading them all is what
    // lets a rebuilt machine be pointed at the root and find the backup
    // of the node it is replacing.
    let mut found: Vec<(PathBuf, Manifest)> = std::fs::read_dir(root.join("nodes"))
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|node| {
            std::fs::read_dir(node.path())
                .into_iter()
                .flatten()
                .flatten()
        })
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
pub fn keeping(
    taken: &[(PathBuf, Manifest)],
    now: i64,
    keep_days: i64,
) -> (Vec<&PathBuf>, Vec<&PathBuf>) {
    let horizon = now - keep_days.max(1) * 24 * 60 * 60 * 1000;
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

/// The copy a restore is about to bring up.
///
/// Rows only: it unpacks and replays at its next deployment, so what this
/// returns is what somebody has to be told — which backup, from when, and
/// where it lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    /// So a caller that can deploy — the console — can start it without
    /// looking the service up again by a name it just chose.
    pub service_id: String,
    pub slug: String,
    /// Where the volume comes from, as it is stored on the row: a path
    /// here, or `s3://…` / `ssh://…` when the backup is not on this
    /// machine any more. The deployment fetches a remote one before it
    /// unpacks — see `Deployer::prepare_engine`.
    pub from: String,
    pub taken_at: i64,
    pub target: Option<String>,
}

/// Why a restore did not start.
///
/// Typed rather than a string, because two of these have a way out that
/// only a terminal can act on and the console must not print a sentence
/// about a flag it does not have. Each `Display` is one sentence: the
/// console puts it in a strip, and `restore` prints the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotRestored {
    /// No managed database of that name on this node.
    NoSuchDatabase(String),
    /// Not a moment anybody could have meant.
    NotAMoment(String),
    /// A path that holds no `manifest.json`.
    NotABackup(PathBuf),
    /// The named backup was taken *after* the moment asked for.
    NamedIsNewer(PathBuf),
    /// Nothing on this node was taken before that moment.
    NothingOldEnough(PathBuf),
    /// A backup old enough, holding no copy of *this* database.
    HoldsNoCopy(PathBuf),
    /// The rows could not be written.
    Refused(String),
}

impl std::fmt::Display for NotRestored {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchDatabase(slug) => write!(f, "no database called {slug:?} on this node"),
            Self::NotAMoment(text) => {
                write!(f, "{text:?} is not a moment. Try 2026-08-16 14:32")
            }
            Self::NotABackup(path) => {
                write!(
                    f,
                    "{} is not a backup: no manifest.json in it",
                    path.display()
                )
            }
            Self::NamedIsNewer(path) => write!(
                f,
                "{} was taken after that moment, so it cannot reach it",
                path.display()
            ),
            Self::NothingOldEnough(path) => write!(
                f,
                "no backup in {} was taken before that moment",
                path.display()
            ),
            Self::HoldsNoCopy(path) => {
                write!(f, "{} holds no copy of this database", path.display())
            }
            Self::Refused(reason) => write!(f, "{reason}"),
        }
    }
}

/// Restore **one database** into a copy beside it, with the caller's
/// database handle.
///
/// The one implementation, and both doors are thin over it: the command
/// above and the form on the database's own page. Jorge asked for the
/// second — the window card said "any moment between these two" and there
/// was nowhere to go with that.
///
/// **A copy, never the original rewound.** The original goes on serving,
/// which is what makes this a thing somebody can do at all: a restore
/// that replaced the live database would be a decision nobody can take
/// back, and comparing the two is most of what a recovery actually is.
///
/// Nothing is deployed here. The rows say what to unpack and where to
/// stop; the deployment that does it belongs to whoever asked — which for
/// the console is the job it queues next, and for the command is the
/// operator, who is told.
pub async fn restore_one(
    config: &Config,
    database: &wabot::sqlite::SqliteDatabase,
    service_slug: &str,
    target: Option<&str>,
    into: Option<&str>,
    from: Option<&Path>,
) -> Result<Restored, NotRestored> {
    let refused =
        |error: crate::domain::errors::PlatformError| NotRestored::Refused(error.to_string());

    let services = crate::infra::repositories::services::all(database, None)
        .await
        .map_err(refused)?;
    let Some(source) = services
        .iter()
        .find(|service| service.slug == service_slug && service.kind.is_managed())
    else {
        return Err(NotRestored::NoSuchDatabase(service_slug.to_string()));
    };
    let Some(row) = crate::infra::repositories::databases::of_service(database, &source.id)
        .await
        .map_err(refused)?
    else {
        return Err(NotRestored::NoSuchDatabase(service_slug.to_string()));
    };
    let Some(project) = crate::infra::repositories::projects::find(database, &source.project_id)
        .await
        .map_err(refused)?
    else {
        return Err(NotRestored::NoSuchDatabase(service_slug.to_string()));
    };

    // Which backup can reach that moment. Anything else is a restore
    // that lands somewhere the operator did not ask for — see
    // `base_for`, where the direction of replay is the whole reason.
    let at = match target {
        Some(text) => {
            parse_target(text).ok_or_else(|| NotRestored::NotAMoment(text.to_string()))?
        }
        None => crate::infra::clock::now_ms(),
    };
    // A backup somebody names, or the ones this node keeps.
    //
    // **Both, because a backup that has been moved is still a backup.**
    // The first version looked only in `backups/`, so taking one with
    // `--out /root/somewhere` and then trying to restore from it was
    // answered with "no backup on this node was taken before that
    // moment" — true, and read in the middle of a recovery as "you have
    // nothing", by somebody holding the thing in their hand. Found
    // doing exactly that.
    let named: Option<(PathBuf, Manifest)> = from.and_then(|path| {
        let manifest = std::fs::read_to_string(path.join("manifest.json")).ok()?;
        Some((
            path.to_path_buf(),
            serde_json::from_str::<Manifest>(&manifest).ok()?,
        ))
    });
    if let Some(path) = from {
        if named.is_none() {
            return Err(NotRestored::NotABackup(path.to_path_buf()));
        }
    }

    let container = format!("{}.{}", project.slug, source.slug);

    // A backup somebody named is the whole answer: they are holding it,
    // and its two failures are about *it* rather than about what this
    // node has.
    let (unpacks, taken_at) = match named {
        Some((path, manifest)) => {
            if manifest.taken_at > at {
                return Err(NotRestored::NamedIsNewer(path));
            }
            if !path
                .join("volumes")
                .join(&container)
                .join("base.tar.gz")
                .exists()
            {
                return Err(NotRestored::HoldsNoCopy(path));
            }
            (
                Destination::Local(path.join("volumes").join(&container)),
                manifest.taken_at,
            )
        }
        None => {
            // Here first, because a copy on this disk costs nothing to
            // read. Only the backups whose volume is really on the disk:
            // a manifest is a claim, and for a local one the file is the
            // fact.
            let here: Vec<(PathBuf, Manifest)> = taken(&config.node.data_dir)
                .into_iter()
                .filter(|(path, _)| {
                    path.join("volumes")
                        .join(&container)
                        .join("base.tar.gz")
                        .exists()
                })
                .collect();

            match base_for(&here, at, Some(&container)) {
                Some((path, manifest)) => (
                    Destination::Local(path.join("volumes").join(&container)),
                    manifest.taken_at,
                ),
                // **Then wherever this node sends its backups.** With a
                // destination there is nothing here at all: the local
                // copy is removed the moment the transfer succeeds, which
                // is the whole point of having sent it. So a node that
                // had been backing itself up all week would be told by
                // the machine that took them that it has none. Jorge
                // asked exactly this question.
                None => {
                    let plan = crate::infra::backup::plan::plan(database).await;
                    // Anywhere that is not the root just searched — a
                    // bucket, another machine, or a second disk here.
                    // Comparing against the root rather than asking
                    // `is_remote` is the difference between "somewhere
                    // else" and "on another machine", and only the first
                    // is the question.
                    let elsewhere = plan
                        .destination
                        .as_deref()
                        .and_then(|text| Destination::parse(text).ok())
                        .filter(|destination| {
                            destination != &Destination::Local(config.node.data_dir.join("backups"))
                        });
                    let Some(destination) = elsewhere else {
                        return Err(refusal(&here, at, config.node.data_dir.join("backups")));
                    };
                    // The manifests only — a listing and a few hundred
                    // bytes each. What is downloaded is one volume,
                    // later, by the deployment that unpacks it.
                    let there = crate::infra::backup::destination::index(&destination, config)
                        .await
                        .map_err(NotRestored::Refused)?;
                    match base_for(&there, at, Some(&container)) {
                        Some((at_destination, manifest)) => (
                            at_destination.child(&format!("volumes/{container}")),
                            manifest.taken_at,
                        ),
                        None => {
                            return Err(refusal(
                                &there,
                                at,
                                std::path::PathBuf::from(destination.display()),
                            ))
                        }
                    }
                }
            }
        }
    };

    let name = into
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}-restored", source.slug));
    let (restored, _) = crate::infra::backup::restore_database_into::restore_into(
        database,
        &row,
        &project.id,
        &name,
        source
            .memory_limit
            .unwrap_or(crate::domain::policies::presets::SMALLEST),
        &unpacks.display(),
        target,
    )
    .await
    .map_err(refused)?;

    Ok(Restored {
        service_id: restored.id,
        slug: restored.slug,
        from: unpacks.display(),
        taken_at,
        target: target.map(str::to_string),
    })
}

/// Whether a backup's manifest claims a copy of this database.
///
/// The manifest and not the disk, so the same question can be asked of a
/// bucket without downloading anything: `back_up_engines` writes one
/// entry per volume it copied, and a database it could not copy is
/// **absent** from that list. That is what makes "this backup holds it"
/// answerable before a single byte moves.
fn holds(manifest: &Manifest, container: &str) -> bool {
    manifest
        .volumes
        .iter()
        .any(|copied| copied.container == container)
}

/// Read a moment as somebody would type it: `2026-08-16 14:32`.
///
/// UTC, because the server parses `recovery_target_time` in its own time
/// zone and that is UTC here — and because the console labels every time
/// it shows for exactly this reason.
pub fn parse_target(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, clock) = text.split_once([' ', 'T'])?;
    let mut parts = date.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;

    let mut clock = clock.split(':');
    let hour: u8 = clock.next()?.parse().ok()?;
    let minute: u8 = clock.next()?.parse().ok()?;
    let second: u8 = clock.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let date =
        time::Date::from_calendar_date(year, time::Month::try_from(month).ok()?, day).ok()?;
    let clock = time::Time::from_hms(hour, minute, second).ok()?;
    Some(date.with_time(clock).assume_utc().unix_timestamp() * 1000)
}

/// Copy every volume, each the way its owner allows.
async fn copy_volumes(
    database: &wabot::sqlite::SqliteDatabase,
    config: &Config,
    into: &Path,
) -> Vec<Copied> {
    let root = crate::infra::repositories::volumes::root(&config.node.data_dir);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };

    // What this node's rows claim, and which of those needs its own
    // tool. Both read from the rows rather than guessed from the
    // directory name: the name carries a project and a service and says
    // nothing about whether anything still runs there or what is
    // inside.
    let claimed = crate::infra::deploy::Deployer::claimed(database)
        .await
        .map(|claims| {
            claims
                .into_iter()
                .map(|claim| (claim.container, claim.managed))
                .collect::<std::collections::HashMap<_, _>>()
        });
    let mut copied = Vec::new();
    let mut orphans = Vec::new();

    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let container = entry.file_name().to_string_lossy().to_string();
        let destination = into.join("volumes").join(&container);

        // **Storage no copy claims is not backed up.** It is data
        // somebody may still want — which is why nothing deletes it —
        // but it is not part of what this node *is*: there is no row to
        // put it back under, so it would be weight in every backup for
        // ever with no restore behind it. 62 MB of a database that had
        // been moved off the Alpine node, in each copy, found by
        // watching one run.
        //
        // Named rather than skipped quietly, because "the backup has
        // everything on the disk" is what somebody would otherwise
        // assume, and finding out it did not is the one moment when
        // being wrong about a backup costs the most.
        //
        // A managed engine's data directory is copied by the engine —
        // see the module docs — and that happens below, per service,
        // because it needs the rows to know which copy to read from.
        match take(&container, claimed.as_ref()) {
            Take::Copy => {}
            Take::Engine => continue,
            Take::Skip => {
                orphans.push(container);
                continue;
            }
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

    if !orphans.is_empty() {
        println!(
            "  volumes     {} skipped, claimed by no copy of any service:",
            orphans.len()
        );
        for orphan in &orphans {
            println!("                {orphan}");
        }
        println!("              Yours to keep or remove; `doctor` lists them too.");
    }
    copied
}

/// Keep the images nothing else has.
///
/// **The node is the registry.** A pushed image lives in this node's own
/// containerd content store and nowhere else — so `docker.io/...` comes
/// back with a pull and `<this node>/project/service` does not. A backup
/// that skipped these would restore a node whose every pushed service
/// asks itself for an image it no longer holds, and fails at the pull.
///
/// **Only what a service is running.** Not every tag ever pushed, which
/// is a registry's job and grows without bound. Restoring a node is
/// coming back up, and coming back up needs the image each service would
/// deploy. What a CI system pushed last year is what a CI system has.
///
/// Blobs go to the shared store under their digest, so a network of
/// nodes running the same base image keeps it once.
async fn keep_images(database: &wabot::sqlite::SqliteDatabase, root: &Path) -> Vec<Kept> {
    let Some(domain) = crate::infra::node::settings::domain(database, &Config::default()).await
    else {
        // With no name of its own this node has no registry anybody
        // pushed to, so there is nothing here that a pull cannot get.
        return Vec::new();
    };
    let Ok(services) = crate::infra::repositories::services::all(database, None).await else {
        return Vec::new();
    };
    let Ok(client) = crate::infra::runtime::client::Containerd::connect().await else {
        println!("  images      containerd is not answering; none kept");
        return Vec::new();
    };

    let mut kept: Vec<Kept> = Vec::new();
    let (mut written, mut shared) = (0usize, 0usize);

    for service in &services {
        // What it would actually deploy: the release that is running,
        // or the image on the service when nothing has been pushed.
        let reference =
            match crate::infra::repositories::releases::of_service(database, &service.id).await {
                Ok(releases) => releases
                    .into_iter()
                    .find(|release| release.deployed_at.is_some())
                    .map(|release| release.reference)
                    .unwrap_or_else(|| service.image.clone()),
                Err(_) => service.image.clone(),
            };

        // Ours or somebody's. A public image is somebody else's to keep,
        // and copying it here would be gigabytes of what a pull returns.
        if !reference.starts_with(&format!("{domain}/")) {
            continue;
        }
        if kept.iter().any(|one| one.reference == reference) {
            continue;
        }

        match keep_one(&client, root, &reference).await {
            Ok((one, new, old)) => {
                println!(
                    "  image       {reference}  {} blob(s), {new} new",
                    one.blobs.len()
                );
                written += new;
                shared += old;
                kept.push(one);
            }
            // Named and skipped, like a database that could not be
            // copied: a backup missing one image is one somebody can
            // still use for everything else.
            Err(reason) => println!("  image       {reference} — NOT KEPT: {reason}"),
        }
    }

    // **Said either way.** Nothing kept is a real and common answer — a
    // node whose services all run public images has nothing here that a
    // pull cannot get — and it is indistinguishable from a bug that kept
    // nothing, unless the command says which. "My images are in the
    // backup" is what somebody assumes by default.
    match kept.is_empty() {
        true => println!(
            "  images      none of this node's own are in use; public ones come back with a pull"
        ),
        false => println!(
            "  images      {} kept, {written} blob(s) written, {shared} already shared",
            kept.len()
        ),
    }
    kept
}

/// One image: its tree, its bytes, and where they went.
async fn keep_one(
    client: &crate::infra::runtime::client::Containerd,
    root: &Path,
    reference: &str,
) -> Result<(Kept, usize, usize), String> {
    let target = crate::infra::runtime::images::image_target(client, reference)
        .await
        .map_err(|error| error.to_string())?;
    let blobs = crate::infra::backup::blobs::tree_of(client, reference).await?;

    let (mut written, mut shared) = (0usize, 0usize);
    for (digest, size) in &blobs {
        // Asked before read: the point of a shared store is that the
        // second node does not move the bytes at all, and reading a
        // layer out of containerd to discover it is already there would
        // be the copy this exists to avoid.
        if crate::infra::backup::blobs::have(root, digest) {
            shared += 1;
            continue;
        }
        let descriptor = containerd_client::types::Descriptor {
            media_type: String::new(),
            digest: digest.clone(),
            size: *size,
            annotations: Default::default(),
        };
        let bytes = crate::infra::runtime::content::read(client, &descriptor)
            .await
            .map_err(|error| format!("{digest}: {error}"))?;
        match crate::infra::backup::blobs::put(root, digest, &bytes) {
            Ok(true) => written += 1,
            Ok(false) => shared += 1,
            Err(error) => return Err(format!("{digest}: {error}")),
        }
    }

    Ok((
        Kept {
            reference: reference.to_string(),
            digest: target.digest,
            media_type: target.media_type,
            size: target.size,
            blobs,
        },
        written,
        shared,
    ))
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
) -> Engines {
    let (Ok(services), Ok(projects)) = (
        crate::infra::repositories::services::all(database, None).await,
        crate::infra::repositories::projects::all(database).await,
    ) else {
        // Unknown is not empty — the shape this file already keeps for
        // what a volume claims. Rows that cannot be read say nothing
        // about whether a database was copied, so this is not a miss;
        // the copies simply do not happen and the run says so above.
        return Engines {
            copied: Vec::new(),
            missed: vec!["the service rows could not be read".into()],
        };
    };
    let deployer =
        crate::infra::deploy::Deployer::new(std::sync::Arc::new(database.clone()), config);
    let mut copied = Vec::new();
    let mut missed = Vec::new();

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
            Ok(Some(bytes)) => {
                println!("  database    {container}  {}", human(bytes));
                copied.push(Copied {
                    container,
                    bytes,
                    // The engine's own tool wrote it, which is what
                    // makes this the one kind of copy that restores.
                    how: How::Consistent,
                });
            }
            // Somebody else's to copy. Said rather than skipped
            // silently: a database on the page and absent from the
            // backup is exactly the thing an operator would otherwise
            // find out about during a restore.
            Ok(None) => println!("  database    {container} — runs elsewhere, and is that node's"),
            Err(reason) => {
                println!("  database    {container} — NOT COPIED: {reason}");
                // Left on disk rather than removed: a half-written
                // backup that somebody can look at beats one this
                // command tidied away.
                missed.push(format!("{container} ({reason})"));
            }
        }
    }
    Engines { copied, missed }
}

/// What `back_up_engines` did, and what it could not do.
///
/// The two are kept apart because they answer different questions: the
/// copies go in the manifest, and the misses decide whether this backup
/// **worked**. Counting instead — managed services against volumes
/// copied — was the version that reads right and is wrong: a database
/// placed on another node is not a copy this run owed anybody, and it
/// would have made every backup on such a node report a shortfall for
/// ever.
struct Engines {
    copied: Vec<Copied>,
    /// One per managed database **on this node** that could not be
    /// copied, with the reason.
    missed: Vec<String>,
}

/// What a backup does with one directory under the volume root.
#[derive(Debug, PartialEq, Eq)]
enum Take {
    /// A file copy, here and now. The manifest calls it
    /// crash-consistent, in those words.
    Copy,
    /// The engine that owns it copies it — later, and per service,
    /// because that needs the rows to know which copy to read from.
    Engine,
    /// Left behind, and named. See the call site for why.
    Skip,
}

/// Which of the three, given what this node's rows claim.
///
/// `None` for the claims is not "nothing is claimed". Rows this node
/// could not read are no grounds for deciding somebody's data is
/// rubbish, so with no list everything is claimed — the shape `fits`
/// takes for a machine that cannot say how much memory it has.
fn take(container: &str, claimed: Option<&std::collections::HashMap<String, bool>>) -> Take {
    match claimed {
        None => Take::Copy,
        Some(claimed) => match claimed.get(container) {
            Some(true) => Take::Engine,
            Some(false) => Take::Copy,
            None => Take::Skip,
        },
    }
}

/// Copy a directory, and say how much it holds.
///
/// Symlinks are not followed, for the reason `disk::used` gives: a
/// volume that pointed at `/` would otherwise copy the machine.
pub(crate) fn copy_tree(from: &Path, to: &Path) -> std::io::Result<u64> {
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
    crate::infra::node::memory::human(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(entries: &[(&str, bool)]) -> std::collections::HashMap<String, bool> {
        entries
            .iter()
            .map(|(id, managed)| (id.to_string(), *managed))
            .collect()
    }

    /// A directory no copy of any service claims is left where it is.
    ///
    /// It is data somebody may still want, which is why nothing deletes
    /// it — and it is not part of what this node is, so carrying it in
    /// every backup for ever would be weight with no restore behind it:
    /// there is no row to put it back under. The Alpine node had 62 MB
    /// of a database that had been moved off it, copied every time.
    #[test]
    fn a_volume_nothing_claims_is_not_backed_up() {
        let claimed = claims(&[("shop.web.1", false), ("shop.db.1", true)]);

        assert_eq!(take("shop.web.1", Some(&claimed)), Take::Copy);
        assert_eq!(take("shop.db.1", Some(&claimed)), Take::Engine);
        assert_eq!(take("gone.orders.3", Some(&claimed)), Take::Skip);
    }

    /// Rows this node could not read are no grounds for deciding
    /// somebody's data is rubbish.
    ///
    /// The dangerous reading of an empty list is "nothing is claimed",
    /// which would turn one unreadable query into a backup that
    /// silently holds no volumes at all — and looks like a backup. So
    /// unknown means everything is copied, which is what this command
    /// did before it could tell the difference.
    #[test]
    fn rows_that_cannot_be_read_mean_copy_everything() {
        assert_eq!(take("shop.web.1", None), Take::Copy);
        assert_eq!(take("gone.orders.3", None), Take::Copy);

        // And an empty list is a different answer from a missing one:
        // this node genuinely runs nothing, so nothing is claimed.
        let none = claims(&[]);
        assert_eq!(take("gone.orders.3", Some(&none)), Take::Skip);
    }

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
                    images: Vec::new(),
                },
            )
        };
        // Newest first, as `taken` returns them.
        let backups = vec![at(10 * day), at(5 * day), at(day)];

        // A moment between two backups starts from the earlier one.
        let picked = base_for(&backups, 7 * day, None).expect("one before it");
        assert_eq!(picked.1.taken_at, 5 * day);

        // A moment after everything starts from the newest.
        assert_eq!(
            base_for(&backups, 20 * day, None)
                .expect("the newest")
                .1
                .taken_at,
            10 * day
        );

        // Exactly at a backup is that backup: it holds that moment.
        assert_eq!(
            base_for(&backups, 5 * day, None)
                .expect("that one")
                .1
                .taken_at,
            5 * day
        );

        // And a moment before anything kept has no answer, rather than
        // silently getting one that is too late.
        assert!(base_for(&backups, 12 * 60 * 60 * 1000, None).is_none());
        let none: [(PathBuf, Manifest); 0] = [];
        assert!(base_for(&none, 5 * day, None).is_none());
    }

    /// A moment is read the way somebody types it, and refused rather
    /// than guessed at. A restore that silently landed on the wrong day
    /// because a string was misread is the failure with no symptom.
    #[test]
    fn a_moment_reads_the_way_somebody_types_it() {
        // Both separators, because one is what a person writes and the
        // other is what a machine hands over.
        let noon = parse_target("2026-08-16 12:00").expect("a moment");
        assert_eq!(parse_target("2026-08-16T12:00"), Some(noon));
        // Seconds are optional, and default to the start of the minute —
        // "restore to 14:32" means the beginning of that minute.
        assert_eq!(parse_target("2026-08-16 12:00:00"), Some(noon));
        assert_eq!(parse_target("2026-08-16 12:00:30"), Some(noon + 30_000));

        // UTC, which is what the server parses it as. The number is
        // checked against an independent implementation rather than
        // written from memory — the first version of this line was a
        // day out, and a date constant that is wrong in a test is a
        // test that agrees with the bug.
        assert_eq!(noon, 1_786_881_600_000);

        for nonsense in [
            "yesterday",
            "2026-08-16",
            "2026-13-01 00:00",
            "2026-08-32 00:00",
            "2026-08-16 25:00",
            "",
        ] {
            assert!(parse_target(nonsense).is_none(), "{nonsense:?}");
        }
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
        let archive = crate::infra::deploy::database::archive_dir(dir.path(), "demo.orders");
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
                    images: Vec::new(),
                },
            )
        };

        // Newest first, as `taken` returns them.
        let backups = vec![at(1), at(6), at(9), at(20)];
        let (keep, drop) = keeping(&backups, now, KEEP_DAYS);

        assert_eq!(keep.len(), 3, "and the nine-day-old one anchors the week");
        assert!(keep.iter().any(|path| path.ends_with("backup-9")));
        assert_eq!(drop.len(), 1);
        assert!(drop[0].ends_with("backup-20"));

        // With everything inside the window, nothing anchors it from
        // outside and nothing is dropped: there is no older one to
        // spare.
        let recent = vec![at(1), at(2)];
        let (keep, drop) = keeping(&recent, now, KEEP_DAYS);
        assert_eq!(keep.len(), 2);
        assert!(drop.is_empty());

        // And one backup is never dropped, however old. It is the only
        // thing that could be restored.
        let only = vec![at(400)];
        let (keep, drop) = keeping(&only, now, KEEP_DAYS);
        assert_eq!(keep.len(), 1);
        assert!(drop.is_empty(), "the last backup is not rubbish");
    }

    /// The window is the operator's, and the anchor rule holds at any
    /// width of it.
    ///
    /// It was a constant, which meant a node with a small disk had no
    /// answer but to turn the whole thing off. It is a field on the
    /// node's Backup tab now — see `node::backups` — and this asks the
    /// same question of a shorter one: the backup *before* the window
    /// still stays, because it is what makes the whole of the window
    /// reachable.
    #[test]
    fn a_shorter_window_drops_more_and_still_keeps_its_anchor() {
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
                    images: Vec::new(),
                },
            )
        };
        let backups = vec![at(1), at(6), at(9), at(20)];

        let (keep, drop) = keeping(&backups, now, 2);
        assert_eq!(keep.len(), 2, "yesterday's, and the six-day-old anchor");
        assert!(keep.iter().any(|path| path.ends_with("backup-6")));
        assert_eq!(drop.len(), 2);

        // Nought is not a shorter window — it is every backup expiring
        // the moment it is taken — so the floor holds here as well as on
        // the form.
        let (keep, _) = keeping(&backups, now, 0);
        assert_eq!(
            keep.len(),
            2,
            "a window of nothing is a day, not the empty set"
        );
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
            images: Vec::new(),
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
