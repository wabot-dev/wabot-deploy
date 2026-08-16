//! Which write-ahead log may be thrown away, and which may never be.
//!
//! ## The one rule
//!
//! **A segment may go only when no base backup you are keeping still
//! needs it.** A base backup is consistent from its own start position
//! onwards, so it needs every segment from there to whenever you want to
//! recover to — and deleting one in the middle does not shorten the
//! window, it *puts a hole in it*: recovery stops at the hole, and
//! everything after it is unreachable.
//!
//! That is why this is arithmetic on names rather than a policy on
//! dates. A segment's filename is what says whether some backup needs
//! it, and a rule made of timestamps would be right until a clock or a
//! busy hour made it wrong.
//!
//! ## The names are built to be compared
//!
//! `000000010000000000000013` is a timeline and two halves of a segment
//! number, in fixed-width hex, precisely so that "older than" is a
//! string comparison. Nothing here parses more of it than it must.
//!
//! ## What decides the window
//!
//! The **oldest base backup still kept**. Everything before its start
//! position is unreachable by any restore this node could do, and
//! everything from it onwards is the window an operator is promised. So
//! the two are one decision: dropping a base backup is what makes its
//! log deletable, and this refuses to do the second without the first.

/// How many bytes of write-ahead log one segment holds.
///
/// Postgres's default, and not read from the server: this node never
/// passes `--wal-segsize`, so `initdb` uses the default and every
/// database here has it. A node that met a database built elsewhere
/// with a different size would compute the wrong name — which is why
/// this is a constant with a reason rather than a magic number.
pub const SEGMENT_BYTES: u64 = 16 * 1024 * 1024;

/// Read a `Start-LSN` as Postgres writes it: two hex halves either side
/// of a slash, `0/13000060`.
pub fn lsn(text: &str) -> Option<u64> {
    let (high, low) = text.trim().split_once('/')?;
    let high = u64::from_str_radix(high.trim(), 16).ok()?;
    let low = u64::from_str_radix(low.trim(), 16).ok()?;
    Some((high << 32) | low)
}

/// The segment file that holds this position.
///
/// The name is three fixed-width hex fields: the timeline, then the
/// segment number split into its high and low halves. The split is at
/// however many segments fit in a 4 GB "log id" — 256 of them at the
/// default size — which is the part that looks arbitrary and is not: it
/// is why the low field never exceeds `FF` on a default install, and why
/// somebody reading an archive sees the third field roll over rather
/// than count on for ever.
pub fn segment_name(timeline: u32, lsn: u64) -> String {
    let segment = lsn / SEGMENT_BYTES;
    let per_id = (1u64 << 32) / SEGMENT_BYTES;
    format!(
        "{timeline:08X}{:08X}{:08X}",
        segment / per_id,
        segment % per_id
    )
}

/// Whether this file may be deleted, given the oldest one still needed.
///
/// A plain comparison, because the names are built for it. Anything that
/// is not a segment name — the archive's own `.tmp` leftovers, a file
/// somebody dropped in there — is **kept**, which is the safe direction:
/// this deletes the ability to restore, so what it does not understand
/// it does not touch.
pub fn may_delete(file: &str, oldest_needed: &str) -> bool {
    let name = file.strip_suffix(".gz").unwrap_or(file);
    // 24 hex digits and nothing else. A backup history file
    // (`…​.00000028.backup`) fails this and is kept, which is right:
    // they are small and they are what somebody reads to work out what
    // happened.
    if name.len() != 24 || !name.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    name < oldest_needed
}

/// The start position a `pg_basebackup` manifest records.
///
/// Its own JSON, version 2, and this reads two fields of it. Parsed
/// rather than assumed because it is the number the whole rule rests
/// on: get it wrong in the safe direction and the archive grows, get it
/// wrong in the other and a restore stops at a hole.
///
/// `None` when the manifest cannot be read or does not say. The caller
/// treats that as "this backup needs everything", which is the only
/// answer that cannot destroy a window.
pub fn start_of(manifest: &str) -> Option<(u32, u64)> {
    let value: serde_json::Value = serde_json::from_str(manifest).ok()?;
    let range = value.get("WAL-Ranges")?.as_array()?.first()?;
    let timeline = range.get("Timeline")?.as_u64()? as u32;
    let start = lsn(range.get("Start-LSN")?.as_str()?)?;
    Some((timeline, start))
}

/// Read a segment name back into the two numbers it is made of.
///
/// The inverse of [`segment_name`], and it exists for one reason: to
/// tell whether the archive is *continuous*. A window with a hole in it
/// is not a shorter window, it is two windows with the second
/// unreachable — so the only way to promise "any moment between here and
/// there" is to check that every name between here and there is present.
pub fn segment_of(name: &str) -> Option<(u32, u64)> {
    if name.len() != 24 {
        return None;
    }
    let timeline = u32::from_str_radix(&name[0..8], 16).ok()?;
    let high = u64::from_str_radix(&name[8..16], 16).ok()?;
    let low = u64::from_str_radix(&name[16..24], 16).ok()?;
    let per_id = (1u64 << 32) / SEGMENT_BYTES;
    Some((timeline, high * per_id + low))
}

/// What an archive holds, and whether it holds it without a gap.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Held {
    pub segments: usize,
    pub bytes: u64,
    /// The newest segment's name, which is as recent as any recovery can
    /// reach.
    pub newest: Option<String>,
    /// The first segment that is **missing**, if the run is broken.
    ///
    /// Recovery stops at a hole, so this is not a detail — it is the
    /// real end of the window, and everything archived after it is disk
    /// being spent on something unreachable. A page that showed the
    /// newest segment as the horizon while this was set would be
    /// promising a recovery that fails half-way.
    pub gap: Option<String>,
}

/// Read one archive: how much, how recent, and whether it is whole.
pub fn held(archive: &std::path::Path) -> Held {
    let mut found: Vec<(u32, u64, u64)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(archive) else {
        return Held::default();
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let stem = name.strip_suffix(".gz").unwrap_or(&name);
        let Some((timeline, segment)) = segment_of(stem) else {
            continue;
        };
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        found.push((timeline, segment, size));
    }
    found.sort();

    let mut held = Held {
        segments: found.len(),
        bytes: found.iter().map(|(_, _, size)| size).sum(),
        newest: found
            .last()
            .map(|(timeline, segment, _)| segment_name(*timeline, segment * SEGMENT_BYTES)),
        gap: None,
    };

    // The first name that should be there and is not. Only within one
    // timeline: a promotion starts a new one, and the jump between them
    // is not a hole.
    for pair in found.windows(2) {
        let (timeline, segment, _) = pair[0];
        let (next_timeline, next_segment, _) = pair[1];
        if timeline == next_timeline && next_segment != segment + 1 {
            held.gap = Some(segment_name(timeline, (segment + 1) * SEGMENT_BYTES));
            break;
        }
    }
    held
}

/// What a pass over one archive found and did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pruned {
    pub removed: usize,
    pub freed: u64,
    /// Segments left because some kept backup still needs them. This is
    /// the window, in files.
    pub kept: usize,
}

/// Delete the log no kept backup needs.
///
/// `oldest_needed` is the segment the oldest surviving base backup
/// starts in — see [`start_of`]. **`None` deletes nothing**, and that is
/// the whole safety of this: with no backup to anchor it there is no
/// window, and a pass that "cleaned up" an unanchored archive would
/// throw away the only thing that could have been restored.
///
/// Errors on individual files are counted as kept rather than raised. A
/// segment this could not remove is disk, where a pass that stopped
/// half-way through would be an archive in a state nobody chose.
pub fn prune(archive: &std::path::Path, oldest_needed: Option<&str>) -> Pruned {
    let mut pruned = Pruned::default();
    let Some(oldest) = oldest_needed else {
        return pruned;
    };
    let Ok(entries) = std::fs::read_dir(archive) else {
        return pruned;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        if !may_delete(&name, oldest) {
            pruned.kept += 1;
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => {
                pruned.removed += 1;
                pruned.freed += size;
            }
            Err(error) => {
                tracing::warn!(file = %name, %error, "could not remove an archived segment");
                pruned.kept += 1;
            }
        }
    }
    pruned
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a manifest the node actually wrote, so the shape is
    /// Postgres's rather than mine.
    #[test]
    fn a_manifest_says_where_its_backup_begins() {
        let manifest = r#"{ "PostgreSQL-Backup-Manifest-Version": 2,
            "System-Identifier": 7673202918516576284,
            "Files": [{ "Path": "backup_label", "Size": 227 }],
            "WAL-Ranges": [{ "Timeline": 1, "Start-LSN": "0/13000060",
                             "End-LSN": "0/13000168" }],
            "Manifest-Checksum": "abc" }"#;

        assert_eq!(start_of(manifest), Some((1, 0x13000060)));

        // Unreadable is not "needs nothing" — the caller reads `None`
        // as "needs everything", which is the only answer that cannot
        // put a hole in somebody's recovery window.
        assert_eq!(start_of("{}"), None);
        assert_eq!(start_of("not json"), None);
    }

    /// The three fields are a timeline and a segment number split at
    /// however many fit in a 4 GB log id.
    #[test]
    fn a_position_names_the_segment_that_holds_it() {
        // The real one from the node: 0/13000060 is segment 0x13 of
        // log 0, on timeline 1.
        assert_eq!(segment_name(1, 0x13000060), "000000010000000000000013");
        // The first segment of all.
        assert_eq!(segment_name(1, 0), "000000010000000000000000");
        // And the roll-over, which is the part of the name that looks
        // arbitrary: 256 segments of 16 MB fill one log id.
        assert_eq!(segment_name(1, 0xFF000000), "0000000100000000000000FF");
        assert_eq!(segment_name(1, 0x100000000), "000000010000000100000000");
        assert_eq!(segment_name(2, 0x100000000), "000000020000000100000000");
    }

    /// Older goes, newer stays, and anything this does not understand
    /// stays — because what it deletes is the ability to restore.
    #[test]
    fn only_what_no_kept_backup_needs_is_deletable() {
        let oldest = "000000010000000000000013";

        assert!(may_delete("000000010000000000000012.gz", oldest));
        assert!(may_delete("000000010000000000000001.gz", oldest));
        // The one the backup starts in is needed, not spare.
        assert!(!may_delete("000000010000000000000013.gz", oldest));
        assert!(!may_delete("000000010000000000000014.gz", oldest));

        // Not a segment name: kept, whatever it is.
        assert!(!may_delete(
            "000000010000000000000012.00000028.backup",
            oldest
        ));
        assert!(!may_delete("000000010000000000000012.gz.tmp", oldest));
        assert!(!may_delete("notes.txt", oldest));
        assert!(!may_delete("", oldest));

        // A later timeline sorts after an earlier one, which is what
        // the leading field is for — a promoted database's log must not
        // be deleted by a rule about the log it branched from.
        assert!(!may_delete("000000020000000000000001.gz", oldest));
    }

    /// A name reads back into the numbers it was built from, which is
    /// what makes continuity checkable.
    #[test]
    fn a_segment_name_reads_back() {
        for (timeline, lsn) in [
            (1u32, 0u64),
            (1, 0x13000060),
            (1, 0x100000000),
            (2, 0xFF000000),
        ] {
            let name = segment_name(timeline, lsn);
            let (read_timeline, segment) = segment_of(&name).expect("read back");
            assert_eq!(read_timeline, timeline, "{name}");
            assert_eq!(segment, lsn / SEGMENT_BYTES, "{name}");
        }
        assert_eq!(segment_of("short"), None);
        assert_eq!(segment_of("zzzzzzzzzzzzzzzzzzzzzzzz"), None);
    }

    /// **A hole is the real end of the window.** Recovery stops there,
    /// so everything archived after it is disk spent on something
    /// nothing can reach — and a page showing the newest segment as the
    /// horizon would be promising a recovery that fails half-way.
    #[test]
    fn a_missing_segment_is_where_the_window_really_ends() {
        let archive = tempfile::tempdir().expect("tempdir");
        let write = |name: &str| {
            std::fs::write(archive.path().join(name), vec![0u8; 32]).expect("write");
        };

        write("000000010000000000000010.gz");
        write("000000010000000000000011.gz");
        write("000000010000000000000012.gz");
        let whole = held(archive.path());
        assert_eq!(whole.segments, 3);
        assert_eq!(whole.bytes, 96);
        assert_eq!(whole.newest.as_deref(), Some("000000010000000000000012"));
        assert_eq!(whole.gap, None);

        // 13 never arrives, 14 does: the window ends at 12 whatever the
        // newest one says.
        write("000000010000000000000014.gz");
        let broken = held(archive.path());
        assert_eq!(broken.segments, 4);
        assert_eq!(
            broken.gap.as_deref(),
            Some("000000010000000000000013"),
            "and it names the one that is missing"
        );

        // A new timeline is not a hole: a promotion branches, and the
        // jump between the two is the branch rather than a loss.
        let branched = tempfile::tempdir().expect("tempdir");
        for name in ["000000010000000000000010.gz", "000000020000000000000030.gz"] {
            std::fs::write(branched.path().join(name), vec![0u8; 8]).expect("write");
        }
        assert_eq!(held(branched.path()).gap, None);
    }

    /// **With no backup to anchor it, nothing is deleted.** That is the
    /// safety this rests on: an archive with no surviving base backup
    /// has no recovery window at all, and a pass that tidied it would
    /// throw away the only thing that could ever have been restored —
    /// while looking like it had done its job.
    #[test]
    fn an_archive_with_nothing_to_anchor_it_is_left_alone() {
        let archive = tempfile::tempdir().expect("tempdir");
        for name in ["000000010000000000000001.gz", "000000010000000000000002.gz"] {
            std::fs::write(archive.path().join(name), vec![0u8; 64]).expect("write");
        }

        let pruned = prune(archive.path(), None);
        assert_eq!(pruned.removed, 0);
        assert_eq!(
            std::fs::read_dir(archive.path()).expect("read").count(),
            2,
            "an unanchored archive keeps everything"
        );
    }

    /// The window is what the oldest kept backup needs, and everything
    /// before it is unreachable by any restore this node could do.
    #[test]
    fn what_no_kept_backup_needs_is_freed() {
        let archive = tempfile::tempdir().expect("tempdir");
        let write = |name: &str, size: usize| {
            std::fs::write(archive.path().join(name), vec![0u8; size]).expect("write");
        };
        write("000000010000000000000010.gz", 100);
        write("000000010000000000000011.gz", 100);
        // The one the backup starts in, and the ones after it.
        write("000000010000000000000012.gz", 100);
        write("000000010000000000000013.gz", 100);
        // And something this does not understand, which stays.
        write("000000010000000000000011.00000028.backup", 10);

        let pruned = prune(archive.path(), Some("000000010000000000000012"));
        assert_eq!(pruned.removed, 2);
        assert_eq!(pruned.freed, 200);
        assert_eq!(pruned.kept, 3, "two needed, and one it does not know");

        assert!(!archive.path().join("000000010000000000000011.gz").exists());
        assert!(archive.path().join("000000010000000000000012.gz").exists());
        assert!(archive
            .path()
            .join("000000010000000000000011.00000028.backup")
            .exists());
    }

    /// Two halves of hex, and a slash. Postgres writes it this way
    /// everywhere a position appears.
    #[test]
    fn a_position_reads_as_postgres_writes_it() {
        assert_eq!(lsn("0/13000060"), Some(0x13000060));
        assert_eq!(lsn("1/0"), Some(0x1_00000000));
        assert_eq!(lsn(" 0/D000028 "), Some(0xD000028));
        assert_eq!(lsn("nonsense"), None);
        assert_eq!(lsn("0/"), None);
    }
}
