//! What the archive on disk holds, and taking out what is no longer
//! needed.

use crate::policies::wal::{may_delete, segment_name, segment_of, SEGMENT_BYTES};

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
}
