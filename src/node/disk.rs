//! What is on the disk, and how much of it is left.
//!
//! ## Two questions, and only one of them has a cheap answer
//!
//! How full the filesystem is comes from `statvfs` in one call. How big a
//! volume is does not: volumes share a filesystem, nothing enforces a
//! quota per one, and the kernel keeps no running total for a directory.
//! The only answer is to walk it.
//!
//! ## So the walk is bounded, and says when it gave up
//!
//! A database's data directory is thousands of files and a page render is
//! not the place to walk an unbounded tree. This stops at
//! [`MAX_ENTRIES`] and reports that it stopped, because a number that
//! quietly means "some of it" is worse than one labelled incomplete —
//! this console has been bitten by exactly that before, with a memory
//! reading that counted one container per service.
//!
//! Apparent size, not blocks: `du` reports what the disk gave up and this
//! reports what the files hold. For a Postgres volume they are within a
//! percent, and the number somebody compares against a disk is the one
//! that grows when rows are written.

use std::path::Path;

/// A directory bigger than this is reported as "at least", rather than
/// walked to the end while somebody waits for a page.
const MAX_ENTRIES: usize = 20_000;

/// How much a directory holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub bytes: u64,
    /// Whether the walk stopped early. A figure this is set on is a floor,
    /// and the page says so rather than showing it as the total.
    pub partial: bool,
}

/// Walk `path` and add up what it holds.
///
/// Symlinks are not followed: a volume that pointed at `/` would
/// otherwise walk the machine, and the size of what a link points at is
/// not the size of the volume.
pub fn used(path: &Path) -> Usage {
    let mut usage = Usage::default();
    let mut seen = 0usize;
    let mut stack = vec![path.to_path_buf()];

    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                usage.partial = true;
                return usage;
            }
            // `symlink_metadata`, so a link is counted as the link it is.
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                stack.push(entry.path());
            } else if metadata.is_file() {
                usage.bytes += metadata.len();
            }
        }
    }
    usage
}

/// What the filesystem holding `path` has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Filesystem {
    pub total: u64,
    /// What a process without privileges could still write. Not
    /// `free`: a filesystem reserves blocks for root, and reporting those
    /// as available is how a node says it has room and then cannot write.
    pub available: u64,
}

impl Filesystem {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}

/// Ask the kernel how full the filesystem is.
#[cfg(unix)]
pub fn filesystem(path: &Path) -> Filesystem {
    use std::os::unix::ffi::OsStrExt;

    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return Filesystem::default();
    };
    // Zeroed rather than uninitialised: `statvfs` fills every field it
    // documents, and a partial fill on an error path would be read as
    // numbers rather than as a failure.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c_path` is a NUL-terminated path that outlives the call,
    // and `stat` is a valid, correctly sized destination.
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return Filesystem::default();
    }

    let block = stat.f_frsize as u64;
    Filesystem {
        total: block * stat.f_blocks as u64,
        available: block * stat.f_bavail as u64,
    }
}

#[cfg(not(unix))]
pub fn filesystem(_path: &Path) -> Filesystem {
    Filesystem::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_is_what_its_files_hold() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("one"), vec![0u8; 1000]).expect("write");
        std::fs::create_dir(dir.path().join("nested")).expect("mkdir");
        std::fs::write(dir.path().join("nested").join("two"), vec![0u8; 500]).expect("write");

        let usage = used(dir.path());
        assert_eq!(usage.bytes, 1500, "every file, at any depth");
        assert!(!usage.partial);
    }

    /// A path that is not there is nought rather than an error: a volume
    /// is created by the first deployment, and a page rendered before
    /// that should say "nothing" rather than refuse to render.
    #[test]
    fn a_directory_that_is_not_there_holds_nothing() {
        assert_eq!(used(Path::new("/no/such/volume")), Usage::default());
    }

    /// The filesystem this test is running on has some room and some
    /// total — the point is that the call works and the numbers are
    /// ordered, not what they are on any particular machine.
    #[test]
    fn the_filesystem_answers_and_its_numbers_agree() {
        let fs = filesystem(Path::new("."));
        assert!(fs.total > 0, "a filesystem with no blocks is not one");
        assert!(fs.available <= fs.total);
        assert_eq!(fs.used(), fs.total - fs.available);
    }
}
