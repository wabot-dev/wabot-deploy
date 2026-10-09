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

/// Everything under `path` except one child.
///
/// The node's own data directory holds the volumes, so asking what the
/// node keeps means walking it *without* them. Subtracting one total
/// from the other would be wrong the moment either walk stopped early:
/// two floors do not make a difference.
pub fn used_besides(path: &Path, skip: &Path) -> Usage {
    let mut usage = Usage::default();
    let Ok(entries) = std::fs::read_dir(path) else {
        return usage;
    };
    for entry in entries.flatten() {
        if entry.path() == skip {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            let part = used(&entry.path());
            usage.bytes += part.bytes;
            usage.partial |= part.partial;
        } else if metadata.is_file() {
            usage.bytes += metadata.len();
        }
    }
    usage
}

/// Where containerd unpacks what it pulls.
///
/// Its default, and not read from anywhere: this node installs
/// containerd with the packaged configuration and never writes a
/// `root =` of its own. A machine that moved it reads as a node whose
/// images cost nothing, which is wrong and visibly so — the remainder
/// swallows them — rather than wrong and plausible.
pub const CONTAINERD_ROOT: &str = "/var/lib/containerd";

/// Where the disk went, in three parts and a remainder.
///
/// The same shape as the memory reading and for the same reason: what
/// an operator needs is the order of magnitude of each part — is the
/// platform costing 200 MB or 2 GB — and which of them to go and look
/// at. Unlike memory, these parts do *not* overlap: a file is in one
/// directory. What they can be is incomplete, which each one says.
#[derive(Debug, Clone, Copy, Default)]
pub struct Breakdown {
    pub filesystem: Filesystem,
    /// What services keep: one directory per copy, under `volumes/`.
    pub volumes: Usage,
    /// containerd's own: the images it pulled and the snapshots it
    /// unpacked them into.
    pub images: Usage,
    /// This node's: its database, its certificates, its logs —
    /// everything under the data directory that is not a volume.
    pub node: Usage,
}

impl Breakdown {
    /// What the three parts hold between them.
    pub fn accounted(&self) -> u64 {
        self.volumes.bytes + self.images.bytes + self.node.bytes
    }

    /// The rest of what the filesystem says is used: the kernel, the
    /// distribution, and whatever else lives on this machine.
    ///
    /// A remainder, never a measurement — and it is the one that grows
    /// when a walk stopped early, which is why the page says when one
    /// did.
    pub fn rest(&self) -> u64 {
        self.filesystem.used().saturating_sub(self.accounted())
    }

    /// Whether any of the walks gave up. A page showing this has three
    /// figures that are floors and a remainder that is too big.
    pub fn partial(&self) -> bool {
        self.volumes.partial || self.images.partial || self.node.partial
    }

    pub fn percent_of_total(&self, bytes: u64) -> f64 {
        match self.filesystem.total {
            0 => 0.0,
            total => (bytes as f64 / total as f64) * 100.0,
        }
    }
}

/// Read all three, plus what the filesystem says.
pub fn breakdown(data_dir: &Path, containerd_root: &Path) -> Breakdown {
    let volumes_root = data_dir.join("volumes");
    Breakdown {
        filesystem: filesystem(data_dir),
        volumes: used(&volumes_root),
        images: used(containerd_root),
        node: used_besides(data_dir, &volumes_root),
    }
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

    /// The node's own figure must not count the volumes, and the way to
    /// get that wrong is to subtract one walk from another: a walk that
    /// stopped early is a floor, and the difference of two floors is
    /// neither.
    #[test]
    fn what_the_node_keeps_leaves_out_what_the_services_do() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data = dir.path();
        std::fs::create_dir_all(data.join("db")).expect("db");
        std::fs::write(data.join("db").join("node.db"), vec![0u8; 4096]).expect("write");
        std::fs::create_dir_all(data.join("volumes").join("demo.orders").join("data"))
            .expect("volume");
        std::fs::write(
            data.join("volumes")
                .join("demo.orders")
                .join("data")
                .join("base"),
            vec![0u8; 8192],
        )
        .expect("write");

        let volumes = used(&data.join("volumes"));
        assert_eq!(volumes.bytes, 8192);

        let node = used_besides(data, &data.join("volumes"));
        assert_eq!(node.bytes, 4096, "the volume is not the node's");
        assert_eq!(used(data).bytes, 12288, "and together they are the tree");
    }

    /// "Everything else" is what the filesystem says minus what was
    /// found, and never below nought — a walk that stopped early makes
    /// the parts smaller, not the remainder negative.
    #[test]
    fn the_remainder_is_what_the_filesystem_says_less_what_was_found() {
        let breakdown = Breakdown {
            filesystem: Filesystem {
                total: 1000,
                available: 400,
            },
            volumes: Usage {
                bytes: 100,
                partial: false,
            },
            images: Usage {
                bytes: 200,
                partial: true,
            },
            node: Usage {
                bytes: 50,
                partial: false,
            },
        };

        assert_eq!(breakdown.accounted(), 350);
        assert_eq!(breakdown.rest(), 250);
        assert!(breakdown.partial(), "one of the walks gave up");

        let overshot = Breakdown {
            volumes: Usage {
                bytes: 5000,
                partial: false,
            },
            ..breakdown
        };
        assert_eq!(overshot.rest(), 0, "and never wraps");
    }

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
