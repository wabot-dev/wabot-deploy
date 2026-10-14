//! `wabot-deploy backup`, `restore` and `restore-node`: the terminal's
//! side of a backup. What a backup is, and how one is taken, is
//! `crate::infra::backup`.

use crate::infra::backup::destination::Destination;
use crate::infra::backup::{copy_tree, once, restore_one, Manifest, NotRestored, FORMAT};
use crate::infra::config::Config;
use std::path::{Path, PathBuf};

/// The command: open the database, take one, and turn how it went into
/// an exit code.
///
/// Thin over [`once`] on purpose. A backup is a scheduled job as well as
/// a command now — the loop in `serve` takes the same one, with the
/// running process's own handle — and two implementations of "what a
/// backup is" would drift the day one of them learned something.
pub async fn run(config: Config, into: Option<String>) -> anyhow::Result<i32> {
    let database = crate::infra::db::open(&config.database_path()).await?;
    let at = crate::infra::clock::now_ms();
    let outcome = once(&config, &database, into).await;
    // Recorded here as well as in `node::backups::take_now`, because a
    // backup somebody typed is a backup of this node: without this the
    // console's Backup tab said "never from here" while cron took one
    // every night, which is a page that lies about the one thing it
    // exists to answer.
    if let Err(error) = crate::infra::backup::plan::record(&database, at, &outcome).await {
        tracing::warn!(%error, "could not record how the backup went");
    }
    // The command owns the handle it opened, so the command closes it.
    // `once` must not: the scheduled run hands it the *node's* handle,
    // and closing that would checkpoint and drop the writer connection
    // the whole process is using.
    database.close().await?;
    match outcome {
        Ok(_) => Ok(0),
        Err(_) => Ok(1),
    }
}

/// Put a whole node back from a backup.
///
/// ## The question it will not answer for you
///
/// **Am I the same node, or a new one?** The id is minted at `install`,
/// kept for ever, and is what every other machine calls this one —
/// along with the WireGuard key, the enrolment secrets and the grants,
/// all of which are in the database this restores. Keeping them makes
/// this machine *be* the node that died: the network never notices.
/// Minting new ones makes it a different node that happens to hold the
/// same data, which then has to re-join while everybody else goes on
/// holding rows about a ghost.
///
/// Both are legitimate — rebuilding a dead machine wants the first,
/// cloning one for a test wants the second — and guessing is not. So it
/// is a flag with no default, and the command refuses without it.
///
/// ## What it will not do
///
/// **Restore onto a node that is already running something.** A node
/// with services of its own is either not the one that died or is one
/// somebody is about to lose. `--over-my-dead-body` is the way to say
/// it anyway, and the current database is copied aside first whatever
/// happens.
pub async fn restore_node(
    config: Config,
    from: String,
    identity: Option<Identity>,
    force: bool,
) -> anyhow::Result<i32> {
    let source = match Destination::parse(&from) {
        Ok(source) => source,
        Err(reason) => {
            println!("{reason}");
            return Ok(1);
        }
    };

    // A remote backup is brought here first — into the same
    // `<root>/nodes/<id>/<moment>` shape, because the restore finds the
    // shared blob store by walking up from it.
    let brought = config
        .node
        .data_dir
        .join("backups")
        .join(format!(".fetched-{}", crate::infra::clock::now_ms()));
    if source.is_remote() {
        println!("fetching {}", source.display());
    }
    let from = match crate::infra::backup::destination::fetch(&source, &brought, &config).await {
        Ok(path) => path,
        Err(reason) => {
            println!("  could not fetch it: {reason}");
            let _ = std::fs::remove_dir_all(&brought);
            return Ok(1);
        }
    };

    let Ok(text) = std::fs::read_to_string(from.join("manifest.json")) else {
        println!("{} is not a backup: no manifest.json in it", from.display());
        return Ok(1);
    };
    let manifest: Manifest = match serde_json::from_str(&text) {
        Ok(manifest) => manifest,
        Err(error) => {
            println!(
                "{} has a manifest this version cannot read: {error}",
                from.display()
            );
            return Ok(1);
        }
    };
    if manifest.format > FORMAT {
        println!(
            "that backup was written by a newer wabot-deploy (format {} against {FORMAT}).",
            manifest.format
        );
        println!("Upgrade this node before restoring it.");
        return Ok(1);
    }

    println!("restoring from {}", from.display());
    println!(
        "  taken       {} by {}",
        crate::shell::layouts::layout::exactly(manifest.taken_at),
        manifest.taken_by
    );
    println!(
        "  of node     {} ({})",
        manifest.node_name.as_deref().unwrap_or("unnamed"),
        manifest.node_id.as_deref().unwrap_or("no id")
    );
    println!(
        "  holds       {} volume(s), {} image(s)",
        manifest.volumes.len(),
        manifest.images.len()
    );

    let Some(identity) = identity else {
        println!();
        println!("  It will not guess whether this machine is that node.");
        println!();
        println!("    --same-node  keep its id, keys and grants. The network never notices");
        println!("                 the machine was replaced. This is rebuilding what died.");
        println!("    --new-node   take the data and mint a new identity. It has to join");
        println!("                 again, and the original stays whatever it is.");
        return Ok(1);
    };

    // **Not while the node is running**, and this one is not
    // overridable.
    //
    // The database is put back with `std::fs::copy`, which writes
    // through the *same inode* the live process has open. That process
    // keeps a write-ahead log and a shared-memory index built for the
    // contents that were there a moment ago, and they now describe a
    // different database entirely — so it can write its old pages back
    // over the restored ones, and nothing anywhere says it did.
    //
    // Two smaller faults sit on top of the same cause, and both were
    // seen on a node before this guard existed: the report at the end
    // could not read the names back (`database is locked`), and
    // `--new-node` fails at `forget_self` *after* the copy — the
    // operator asked for a new identity, got the old one, and got an
    // error instead of the sentence telling them to re-join.
    //
    // `--over-my-dead-body` is not accepted here because it answers a
    // different question — "yes, replace the rows I have" — and there
    // is no version of this somebody means. The last line of a restore
    // has always been "start the node", which assumed this all along.
    if let Some(pid) = crate::infra::bootstrap::service::another_instance() {
        println!();
        println!("  wabot-deploy is still running here as pid {pid}, and restoring would");
        println!("  write its database out from under it. Stop it first:");
        println!();
        match crate::infra::bootstrap::service::stop_command() {
            Some(command) => println!("    {command}"),
            None => println!("    (stop the wabot-deploy process however it was started)"),
        }
        println!();
        println!("  A stop returns before the process does — it drains connections");
        println!("  first — so wait for the pid to go before restoring.");
        return Ok(1);
    }

    // What is here now. A node with services of its own is either not
    // the one that died or is one somebody is about to lose.
    let existing = crate::infra::db::open(&config.database_path()).await?;
    let running = crate::infra::repositories::services::all(&existing, None)
        .await
        .map(|services| services.len())
        .unwrap_or(0);
    if running > 0 && !force {
        println!();
        println!("  this node already has {running} service(s). Restoring replaces its");
        println!("  database with the backup's, and those rows go with it.");
        println!();
        println!("  If that is what you mean: --over-my-dead-body");
        return Ok(1);
    }

    // The current database, aside, before anything replaces it. Even a
    // restore somebody asked for twice is one they can be wrong about,
    // and this is the only copy of what is here now.
    let aside = config
        .node
        .data_dir
        .join("backups")
        .join(format!("replaced-{}.db", crate::infra::clock::now_ms()));
    if let Some(parent) = aside.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let destination = aside.to_string_lossy().to_string();
    existing
        .write(move |connection| {
            connection.execute("VACUUM INTO ?1", [destination])?;
            Ok(())
        })
        .await?;
    existing.close().await?;
    // **And dropped, because `close` is a checkpoint and not a close.**
    // It takes `&self`: it empties the reader pool and truncates the
    // write-ahead log, and the *writer* connection lives on inside the
    // handle until the handle goes. So a handle still in scope holds a
    // lock on the file this is about to overwrite, and the next open of
    // that path — the names report at the end — is refused with
    // `database is locked` by this very function.
    //
    // Which is what happened on a node, and it survived two wrong
    // diagnoses first: the running service (stopped it, still locked)
    // and a stale `-wal`/`-shm` beside the file (removed them, still
    // locked). Both were real faults and neither was this one.
    drop(existing);
    println!();
    println!("  what was here is at {}", aside.display());

    // The database, then what its rows describe. In that order: a
    // volume with no rows is bytes nothing can name.
    replace_database(&from.join("node.db"), &config.database_path())?;
    println!("  database    restored");

    let volumes = restore_volumes(&from, &config);
    let images = restore_images(&from, &manifest, &config).await;
    println!("  volumes     {volumes} restored");
    println!("  images      {images} restored");

    // A new identity is the *absence* of the old one: `ensure_self`
    // mints one at the next start, the same way a fresh install does.
    // Done after the copy rather than by editing the backup, so the
    // file on disk is always exactly what was backed up.
    if identity == Identity::New {
        let database = crate::infra::db::open(&config.database_path()).await?;
        crate::infra::network::forget_self(&database).await?;
        database.close().await?;
        println!("  identity    cleared; this node will mint a new one and must re-join");
    } else {
        println!(
            "  identity    kept: this machine is {}",
            manifest.node_id.as_deref().unwrap_or("that node")
        );
    }

    report_names(&config).await;

    // The download is consumed by now — the database is in place, the
    // volumes are unpacked and the blobs are in containerd — so keeping
    // it is a second copy of a backup that already exists where it came
    // from. Left behind on any earlier failure, deliberately, which is
    // every path that returned before here.
    if source.is_remote() {
        match std::fs::remove_dir_all(&brought) {
            Ok(()) => {}
            Err(error) => println!("  fetched     {} left behind: {error}", brought.display()),
        }
    }

    println!();
    println!("  Start the node. Reconciliation brings the services back up.");
    Ok(0)
}

/// Put a database file in place of another one.
///
/// **A SQLite database is three files, and copying one of them over is
/// how you corrupt the other two.** In WAL mode the pages that have not
/// been checkpointed live in `-wal`, and `-shm` is the index into it.
/// `std::fs::copy` onto the main file leaves both of those behind — so
/// the restored database sits next to the *previous* database's
/// write-ahead log, and the next process to open it replays those pages
/// into it. Pages from one database written over another, which is
/// exactly as bad as it sounds and produces a file that opens.
///
/// Found on a node, and it announced itself gently: `database is
/// locked` from the stale `-shm`, on a restore that otherwise said it
/// had worked. The lock was the symptom that happened to be visible.
///
/// Removed before the copy rather than after, so there is no instant at
/// which a complete-looking database has somebody else's log beside it.
fn replace_database(from: &Path, to: &Path) -> std::io::Result<()> {
    for sidecar in ["-wal", "-shm"] {
        let mut path = to.as_os_str().to_owned();
        path.push(sidecar);
        // Absent is the ordinary case: a cleanly closed database has
        // checkpointed and removed both.
        match std::fs::remove_file(PathBuf::from(path)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    std::fs::copy(from, to)?;
    Ok(())
}

/// Where this node's names point, and where this machine is.
///
/// **Shown, not judged.** The one thing a restore cannot verify is the
/// one thing it most changes: a node rebuilt on new hardware has a new
/// address, and every check this codebase has is blind to that.
/// `resolves_here` compares a name against the node's own domain — and
/// that domain still resolves, to the machine that died. Every name
/// agrees with it, every check says `Here`, and nothing arrives.
///
/// It is not made a rule for the reason `deploy::dns` gives: behind NAT
/// a machine sees a private address while the world reaches it at
/// another, so a node that refused to finish a restore over a mismatch
/// would be wrong on every box behind a router. And refusing is the
/// wrong shape anyway — restoring before repointing DNS is a sensible
/// order and often the only possible one, because the names are in the
/// backup.
///
/// So both numbers go on the screen and the operator decides. They know
/// whether there is a NAT in front of this machine; the node does not.
async fn report_names(config: &Config) {
    // Said rather than skipped. The names are the one part of a restore
    // nothing else will mention, so a silent return here is a node that
    // finished quietly and is unreachable — which is the failure this
    // whole report exists to prevent.
    let database = match crate::infra::db::open(&config.database_path()).await {
        Ok(database) => database,
        Err(error) => {
            println!();
            println!("  names       could not be read back: {error}");
            println!("              Check by hand that every name this node answers for");
            println!("              points at this machine before trusting it.");
            return;
        }
    };
    // What this node was chosen to answer for, which is a smaller set
    // than the hostnames it stores: it can own a service that somebody
    // else serves. The same list `acme` orders certificates for, so
    // what is checked here is exactly what will be asked of the world.
    let names = crate::infra::edge::acme::wanted_names(&database).await;
    let _ = database.close().await;

    println!();
    if names.is_empty() {
        println!("  names       none — this node answers for nothing by name, so there");
        println!("              is no DNS record that has to point at this machine.");
        return;
    }

    println!("  names       what has to reach this machine:");
    for name in &names {
        let found = crate::infra::deploy::dns::lookup(name).await;
        let where_to = match found.is_empty() {
            true => "does not resolve".to_string(),
            false => found
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        };
        println!("                {name} → {where_to}");
    }

    match crate::infra::deploy::dns::outbound_address() {
        Some(address) => println!("              this machine goes out from {address}"),
        None => println!("              this machine could not say which address it goes out from"),
    }

    println!();
    println!("              Agreeing is not proof and disagreeing is not a fault —");
    println!("              behind NAT this machine sees a private address while the");
    println!("              world reaches it at another. You know which this is.");
    println!("              If the records still point at the machine this replaced,");
    println!("              move them: until then nothing arrives, and a certificate");
    println!("              cannot be renewed for a name that does not come here.");
}

/// Which node this machine becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identity {
    /// The one the backup was taken from, keys and all.
    Same,
    /// A different one holding the same data.
    New,
}

/// Copy every volume back out of the backup.
fn restore_volumes(from: &Path, config: &Config) -> usize {
    let Ok(entries) = std::fs::read_dir(from.join("volumes")) else {
        return 0;
    };
    let mut restored = 0;

    for entry in entries.flatten() {
        let container = entry.file_name().to_string_lossy().to_string();
        let into =
            crate::infra::repositories::volumes::root(&config.node.data_dir).join(&container);

        // A managed engine's backup is two tarballs, not a directory
        // tree — the deploy path unpacks those, because unpacking is
        // what makes them a data directory. Copied across as they are.
        match copy_tree(&entry.path(), &into) {
            Ok(_) => restored += 1,
            Err(error) => println!("  volume      {container} — NOT RESTORED: {error}"),
        }
    }
    restored
}

/// Put every image back into containerd.
async fn restore_images(from: &Path, manifest: &Manifest, config: &Config) -> usize {
    if manifest.images.is_empty() {
        return 0;
    }
    // The blobs are in the *shared* store at the root, which is two
    // levels above this backup: `<root>/nodes/<id>/<taken at>`.
    let root = from
        .parent()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .map(|path| path.to_path_buf())
        .unwrap_or_else(|| config.node.data_dir.join("backups"));

    let Ok(client) = crate::infra::runtime::client::Containerd::connect().await else {
        println!("  images      containerd is not answering; none restored");
        return 0;
    };

    let mut restored = 0;
    for kept in &manifest.images {
        match crate::infra::backup::blobs::put_back(&client, &root, kept).await {
            Ok(_) => restored += 1,
            Err(reason) => println!("  image       {} — NOT RESTORED: {reason}", kept.reference),
        }
    }
    restored
}

/// Restore one database to a moment, as a new one beside it.
///
/// **Never the original rewound.** Rewinding is irreversible —
/// everything after the chosen moment is gone, which is exactly what
/// somebody hunting one dropped table does not want — and it leaves the
/// standbys ahead of their primary, needing to be seeded again. A copy
/// leaves the original serving: take what you came for and throw the
/// copy away.
///
/// The moment is UTC, spelled the way the console shows it, because the
/// server parses it in its own time zone and that is UTC here.
pub async fn restore(
    config: Config,
    service_slug: String,
    target: Option<String>,
    into: Option<String>,
    from: Option<PathBuf>,
) -> anyhow::Result<i32> {
    let database = crate::infra::db::open(&config.database_path()).await?;
    let outcome = restore_one(
        &config,
        &database,
        &service_slug,
        target.as_deref(),
        into.as_deref(),
        from.as_deref(),
    )
    .await;
    database.close().await?;

    let made = match outcome {
        Ok(made) => made,
        Err(refusal) => {
            println!("{refusal}");
            // The extra sentence a terminal has room for. The console
            // shows the one above in a strip and has a form to correct;
            // somebody at a prompt has neither, and the way out of two of
            // these is a flag they have to be told about.
            match refusal {
                NotRestored::NothingOldEnough(_) => {
                    println!();
                    println!(
                        "The oldest one there is what bounds how far back a restore can reach."
                    );
                    println!();
                    println!(
                        "If you have one somewhere else — `backup --out` writes wherever it is"
                    );
                    println!("told — name it with `--from <path>`.");
                }
                NotRestored::NamedIsNewer(_) => {
                    println!("Replaying only goes forwards, so it already holds whatever you are");
                    println!("trying to undo.");
                }
                _ => {}
            }
            return Ok(1);
        }
    };

    println!("restoring {service_slug} into {}", made.slug);
    println!(
        "  from      {} ({})",
        made.from,
        crate::shell::layouts::layout::exactly(made.taken_at)
    );
    match &made.target {
        Some(target) => println!("  up to     {target} UTC"),
        None => println!("  up to     as far as the archived log goes"),
    }
    println!();
    // Said, because nothing here starts it: this talks to the database,
    // not to the running node, and "at its next deployment" read as
    // something that would happen. On a VM node the copy sat at "Not
    // deployed" waiting for a click nobody had been asked for.
    println!(
        "  Nothing starts it from here: deploy {} from its page, and it",
        made.slug
    );
    println!("  unpacks and replays then. The original is untouched and still");
    println!("  serving — this is a copy beside it.");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A restored database must not have the previous one's log beside
    /// it.
    ///
    /// SQLite in WAL mode keeps uncheckpointed pages in `-wal` and an
    /// index into them in `-shm`. Replacing only the main file leaves
    /// both, and the next process to open the database replays *those*
    /// pages into *this* file — one database's writes over another's,
    /// producing a file that opens and is wrong.
    ///
    /// It shipped, and it announced itself as `database is locked` on a
    /// node: the stale `-shm` was the symptom that happened to be
    /// visible, on a restore that had already printed `database
    /// restored`.
    #[test]
    fn a_restored_database_does_not_keep_the_old_ones_log() {
        let directory = tempfile::tempdir().expect("tempdir");
        let live = directory.path().join("node.db");
        let backup = directory.path().join("backup.db");

        std::fs::write(&live, b"the database that is here now").expect("write");
        std::fs::write(&backup, b"the database being restored").expect("write");
        // What a live WAL-mode database leaves on disk.
        let (wal, shm) = (
            directory.path().join("node.db-wal"),
            directory.path().join("node.db-shm"),
        );
        std::fs::write(&wal, b"pages of the database that is here now").expect("write");
        std::fs::write(&shm, b"the index into them").expect("write");

        replace_database(&backup, &live).expect("replace");

        assert_eq!(
            std::fs::read(&live).expect("read"),
            b"the database being restored"
        );
        assert!(!wal.exists(), "the previous database's log survived");
        assert!(!shm.exists(), "the previous database's index survived");
    }

    /// The ordinary case is that neither sidecar is there, because a
    /// database closed cleanly checkpoints and removes both. Missing is
    /// not a failure.
    #[test]
    fn replacing_a_database_with_no_log_beside_it_is_fine() {
        let directory = tempfile::tempdir().expect("tempdir");
        let live = directory.path().join("node.db");
        let backup = directory.path().join("backup.db");
        std::fs::write(&live, b"here now").expect("write");
        std::fs::write(&backup, b"restored").expect("write");

        replace_database(&backup, &live).expect("replace");
        assert_eq!(std::fs::read(&live).expect("read"), b"restored");
    }
}
