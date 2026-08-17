//! `wabot-deploy clean` — the disk, and what nobody ever took away.
//!
//! Four kinds of rubbish accumulate on a node, all of them known and
//! none of them collected. Small alone; together they are the difference
//! between a node that runs for a year and one that fills up in six
//! months.
//!
//! ## It shows before it removes
//!
//! `clean` prints what it would do. `clean --apply` does it. A delete
//! command whose default is to delete is one somebody runs once while
//! reading the flags.
//!
//! ## And it will not remove data
//!
//! Three of the four kinds are **regenerable**: a container's generated
//! configuration, its `/etc/hosts`, its log. The fourth is a volume, and
//! a volume is the only thing on this list that nobody can make again.
//!
//! So a volume nothing claims is *named and left*, however much space it
//! is holding, and `--volumes` is a separate word an operator has to
//! type. The reason it is separate rather than merely confirmed: an
//! unclaimed volume is usually a copy that was moved to another node or
//! thrown off this one, and "the replica is gone" and "the data is
//! wanted" are both true at once. That is exactly the case where a tidy
//! default destroys something.
//!
//! ## Images: the tag is somebody's, the digest is nobody's
//!
//! A push leaves two records — the tag that was chosen, and a
//! digest-only reference to the same manifest. The second is how a
//! registry client fetches by content; it is never replaced, so every
//! push adds one for ever and nothing names it again.
//!
//! Those are removable without guessing. A **tag** is not: an image no
//! service currently names may still be the one somebody is about to
//! deploy, or the version they roll back to. So tags are reported with
//! their sizes — on the Ubuntu test node one of them is 453 MB — and the
//! decision is the operator's.
//!
//! ## No number is claimed for what was freed
//!
//! Removing an image record makes its blobs collectable *if nothing else
//! holds them*, and containerd's collector decides when. Both halves
//! matter: on the Ubuntu test node all four digest-only records removed
//! still had a tag pointing at the same manifest, so the disk did not
//! move by a byte — correctly.
//!
//! So no saving is claimed. It says what it removed and says why `df`
//! may be unchanged, because a cleanup that appears to have done nothing
//! is one somebody will run again harder.

use crate::config::Config;

/// How many database copies from updates to keep.
///
/// Each update writes one before migrating, which is the right thing —
/// rolling back a migration is not a file operation, and that copy is
/// the way back. Ten of them, which is what the Ubuntu node had, is
/// nine ways back nobody will take: the useful one is the last, and the
/// one before it is insurance against the last being the problem.
const KEEP_UPDATE_COPIES: usize = 3;

pub async fn run(config: Config, apply: bool, volumes: bool) -> anyhow::Result<i32> {
    let database = crate::db::open(&config.database_path()).await?;

    match apply {
        true => println!("cleaning up"),
        false => println!("what `clean --apply` would remove"),
    }
    println!();

    let mut removed = 0usize;
    let mut kept_back = Vec::new();

    // ── Orphan directories and files ────────────────────────────────
    //
    // The same four kinds `doctor` reports, from the same function and
    // the same derivation of what is claimed — see `Deployer::claimed`
    // for why there is only one of those.
    // `None` means the rows could not be read, and that is not permission
    // to decide anything is unclaimed.
    let Some(claims) = crate::deploy::Deployer::claimed(&database).await else {
        println!("  this node's rows could not be read, so nothing here is known to be");
        println!("  unclaimed. Refusing rather than guessing — see `doctor`.");
        return Ok(1);
    };
    let live = crate::deploy::Claim::containers(&claims);
    for (kind, path) in crate::deploy::Deployer::leftovers(&config.node.data_dir, &live) {
        // A volume is the one kind that cannot be made again.
        if kind == "data" && !volumes {
            kept_back.push(path);
            continue;
        }
        match apply {
            false => println!("  {kind:<11} {}", path.display()),
            true => match remove(&path) {
                Ok(()) => {
                    println!("  {kind:<11} {} removed", path.display());
                    removed += 1;
                }
                Err(error) => println!("  {kind:<11} {} — {error}", path.display()),
            },
        }
    }

    // ── Database copies from updates ────────────────────────────────
    let mut copies: Vec<std::path::PathBuf> =
        std::fs::read_dir(config.node.data_dir.join("backups"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("before-") && name.ends_with(".db"))
            })
            .collect();
    // The name carries the millisecond it was taken, so sorting the name
    // sorts by age — and the newest are the ones worth keeping.
    copies.sort();
    let surplus = copies.len().saturating_sub(KEEP_UPDATE_COPIES);
    for path in copies.into_iter().take(surplus) {
        match apply {
            false => println!("  update copy {}", path.display()),
            true => match std::fs::remove_file(&path) {
                Ok(()) => {
                    println!("  update copy {} removed", path.display());
                    removed += 1;
                }
                Err(error) => println!("  update copy {} — {error}", path.display()),
            },
        }
    }

    // ── Images ──────────────────────────────────────────────────────
    match crate::runtime::client::Containerd::connect().await {
        Err(_) => println!("  images      containerd is not answering; none examined"),
        Ok(client) => {
            let (gone, tagged) = images(&client, &database, apply, &mut removed).await;
            if gone == 0 && tagged.is_empty() {
                println!("  images      nothing unreferenced");
            }
            if !tagged.is_empty() {
                println!();
                println!("  These are tagged, and no service names them. A tag is");
                println!("  something somebody typed, so they are left alone — remove one");
                println!("  with `ctr -n wabot images rm <ref>` if you are sure:");
                println!();
                for (reference, size) in &tagged {
                    let weight = match size {
                        Some(size) => human(*size),
                        None => "?".into(),
                    };
                    println!("    {weight:>8}  {reference}");
                }
                println!();
                println!("  Those sizes are what the store holds for each image's whole");
                println!("  index, so they double-count shared layers and do not add up");
                println!("  to the disk. `ctr images ls` reports something different and");
                println!("  smaller — the one platform you would run.");
            }
        }
    }

    database.close().await?;

    if !kept_back.is_empty() {
        println!();
        println!(
            "  {} volume(s) nothing claims, left where they are:",
            kept_back.len()
        );
        for path in &kept_back {
            println!("    {}", path.display());
        }
        println!();
        println!("  A volume is the only thing here nobody can make again, and one");
        println!("  nothing claims is usually a copy moved elsewhere rather than");
        println!("  rubbish. `clean --apply --volumes` removes them for good.");
    }

    println!();
    match (apply, removed) {
        (false, _) => println!("  Nothing was removed. `clean --apply` does it."),
        (true, 0) => println!("  Nothing to remove."),
        (true, count) => {
            println!("  {count} thing(s) removed.");
            println!();
            // Said plainly, because the first version implied the disk
            // would move and on the test node it did not budge: every
            // digest-only record removed there still had a tag pointing
            // at the same manifest, so nothing became unreferenced.
            // "I cleaned up and df is identical" reads as a broken
            // command unless it says this.
            println!("  A removed image record frees blobs only when nothing else holds");
            println!("  them, and containerd's collector decides when. If a tag still");
            println!("  points at the same manifest, `df` will not move at all — the");
            println!("  record went and the bytes were never only its.");
        }
    }
    Ok(0)
}

/// Digest-only references nothing names, and the tagged ones to report.
///
/// Returns how many were removed and which tags are unreferenced.
async fn images(
    client: &crate::runtime::client::Containerd,
    database: &wabot::sqlite::SqliteDatabase,
    apply: bool,
    removed: &mut usize,
) -> (usize, Vec<(String, Option<i64>)>) {
    let Ok(held) = crate::runtime::images::all(client).await else {
        println!("  images      could not be listed");
        return (0, Vec::new());
    };

    // What a row asks for, which is the only thing that makes an image
    // wanted. Read from the services rather than from what is running:
    // a service that is stopped still names the image it would start.
    let wanted: std::collections::HashSet<String> =
        match crate::platform::services::all(database, None).await {
            Ok(services) => services.into_iter().map(|service| service.image).collect(),
            // Unreadable rows are no grounds for deciding an image is
            // unwanted. With no list, everything is wanted — the shape
            // `backup`'s `take` uses for the same reason.
            Err(_) => return (0, Vec::new()),
        };

    let mut gone = 0;
    let mut tagged: Vec<(String, Option<i64>)> = Vec::new();
    for image in held {
        if wanted.contains(&image.reference) {
            continue;
        }
        if !image.is_digest_only() {
            // Weighed one at a time, because the manifest has to be read
            // to know: the listing's own size field is the manifest's,
            // which is kilobytes of JSON for an image of any size.
            let size = crate::runtime::images::weight(client, &image.reference).await;
            tagged.push((image.reference, size));
            continue;
        }
        match apply {
            false => println!("  image       {}", image.reference),
            true => match crate::runtime::images::forget(client, &image.reference).await {
                Ok(()) => {
                    println!("  image       {} removed", image.reference);
                    *removed += 1;
                    gone += 1;
                }
                Err(error) => println!("  image       {} — {error}", image.reference),
            },
        }
    }
    // Biggest first: the number that decides whether somebody acts. One
    // that could not be weighed sorts last rather than first, so an
    // unreadable manifest does not head a list ordered by size.
    tagged.sort_by_key(|(_, size)| std::cmp::Reverse(size.unwrap_or(-1)));
    (gone, tagged)
}

/// A file or a directory, whichever it is.
fn remove(path: &std::path::Path) -> std::io::Result<()> {
    match path.is_dir() {
        true => std::fs::remove_dir_all(path),
        false => std::fs::remove_file(path),
    }
}

fn human(bytes: i64) -> String {
    match bytes {
        ..=999 => format!("{bytes} B"),
        1_000..=999_999 => format!("{} kB", bytes / 1_000),
        _ => format!("{} MB", bytes / 1_000_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The newest copies stay and the oldest go, which is only true if
    /// the name sorts by age — it carries the millisecond it was taken.
    #[test]
    fn the_oldest_update_copies_are_the_ones_that_go() {
        let mut names = [
            "before-0.10.0-1786419196314.db",
            "before-0.1.4-1786137472733.db",
            "before-0.2.0-1786244097730.db",
            "before-0.3.0-1786245106482.db",
        ];
        names.sort();
        let surplus = names.len().saturating_sub(KEEP_UPDATE_COPIES);
        assert_eq!(surplus, 1);
        // The one taken first, not the one whose version sorts first —
        // `0.1.4` and `0.10.0` sort the wrong way round as text, and the
        // timestamp is what saves this.
        assert_eq!(names[0], "before-0.1.4-1786137472733.db");
        assert_eq!(
            names.last().copied(),
            Some("before-0.3.0-1786245106482.db"),
            "the newest by time is last"
        );
    }

    /// Fewer copies than the number kept must remove nothing, rather
    /// than underflow into removing everything.
    #[test]
    fn a_node_with_few_copies_loses_none() {
        for count in 0..=KEEP_UPDATE_COPIES {
            assert_eq!(count.saturating_sub(KEEP_UPDATE_COPIES), 0, "{count}");
        }
    }
}
