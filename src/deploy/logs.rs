//! What a container said before it died.
//!
//! ## Why there was nothing
//!
//! `containers::run` passed no stdio paths, and containerd's default is
//! to discard. That was a deliberate choice with a good reason attached
//! — a FIFO nobody reads fills up and blocks the container's first
//! write — and it left a hole that only showed once the node started
//! running things it had configured *itself*.
//!
//! A managed database is that case. The node writes its command, its
//! arguments, its `pg_hba.conf` and its environment; if Postgres
//! refuses any of them it says so on stderr and exits, and the console
//! showed `Stopped (exit 1)` with nothing else. On a real node a standby
//! sat stopped for an hour that way. When the node authors the
//! configuration, "it failed" without the reason is the node hiding its
//! own mistake.
//!
//! ## A file, not a FIFO
//!
//! The shim understands a `file://` URI and appends to it — no reader
//! required, so the failure mode the original comment warned about
//! cannot happen.
//!
//! ## One run, not a history
//!
//! The file is truncated when the container starts. There is no
//! rotation here and no log retention policy, so an append-only file is
//! a disk leak with a slow fuse; what somebody needs when a container
//! will not stay up is what *this* attempt said. A real log feature —
//! following, searching, keeping — is a bigger thing than this and is
//! not pretending to be it.

use std::path::{Path, PathBuf};

/// Where a container's output goes.
pub fn path(data_dir: &Path, container_id: &str) -> PathBuf {
    data_dir.join("logs").join(format!("{container_id}.log"))
}

/// The URI form containerd's shim wants.
///
/// `file://` with an empty host, so the path begins at the third slash.
/// A bare path is read as the `fifo` scheme, which is the behaviour this
/// module exists to avoid.
pub fn uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// Make the directory and empty the file, ready for a run.
///
/// Truncating rather than appending: see the module docs. Returns the
/// path so the caller can hand it to containerd.
pub fn prepare(data_dir: &Path, container_id: &str) -> std::io::Result<PathBuf> {
    let path = path(data_dir, container_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, b"")?;
    Ok(path)
}

/// The last of what a container said, or `None` if it said nothing.
///
/// Bounded, because this ends up in a database column and on a page: a
/// container in a crash loop can write megabytes, and the useful part of
/// a failure is the end of it.
pub fn tail(data_dir: &Path, container_id: &str, limit: usize) -> Option<String> {
    let contents = std::fs::read_to_string(path(data_dir, container_id)).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return None;
    }

    // From a character boundary, or the string would not be one.
    let start = trimmed
        .char_indices()
        .rev()
        .take(limit)
        .last()
        .map(|(index, _)| index)
        .unwrap_or(0);
    Some(trimmed[start..].to_string())
}

/// Throw away what a container said, when the container itself is going.
pub fn discard(data_dir: &Path, container_id: &str) {
    let path = path(data_dir, container_id);
    if let Err(error) = std::fs::remove_file(&path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(file = %path.display(), %error, "removing a container's log");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `file://` with an empty host. A bare path is the `fifo` scheme to
    /// the shim, which is the thing this avoids.
    #[test]
    fn the_uri_is_one_the_shim_reads_as_a_file() {
        let uri = uri(Path::new("/var/lib/wabot-deploy/logs/demo.db.log"));
        assert_eq!(uri, "file:///var/lib/wabot-deploy/logs/demo.db.log");
        assert!(uri.starts_with("file:///"), "an empty host, then the path");
    }

    #[test]
    fn a_run_starts_from_an_empty_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = prepare(dir.path(), "demo.db").expect("prepared");
        std::fs::write(&path, "the first run said this").expect("write");

        assert_eq!(
            tail(dir.path(), "demo.db", 4096).as_deref(),
            Some("the first run said this")
        );

        // The second run does not inherit it: what somebody needs when a
        // container will not stay up is what *this* attempt said.
        prepare(dir.path(), "demo.db").expect("prepared again");
        assert_eq!(tail(dir.path(), "demo.db", 4096), None);
    }

    /// A crash loop writes megabytes and this ends up in a column and on
    /// a page. The end of a failure is the useful part of it.
    #[test]
    fn the_tail_is_bounded_and_is_the_end() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = prepare(dir.path(), "demo.db").expect("prepared");
        let noise = format!("{}\nthe last line", "x".repeat(10_000));
        std::fs::write(&path, noise).expect("write");

        let tail = tail(dir.path(), "demo.db", 100).expect("something");
        assert!(tail.len() <= 100, "{}", tail.len());
        assert!(tail.ends_with("the last line"), "{tail}");
    }

    /// Multi-byte output must not be cut through a character.
    #[test]
    fn the_tail_starts_on_a_character_boundary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = prepare(dir.path(), "demo.db").expect("prepared");
        std::fs::write(&path, "á".repeat(200)).expect("write");

        let tail = tail(dir.path(), "demo.db", 50).expect("something");
        assert!(tail.chars().all(|c| c == 'á'), "cut through a character");
    }

    #[test]
    fn nothing_said_is_nothing_shown() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(tail(dir.path(), "never-ran", 4096), None);

        prepare(dir.path(), "demo.db").expect("prepared");
        assert_eq!(tail(dir.path(), "demo.db", 4096), None, "an empty file");

        std::fs::write(path(dir.path(), "demo.db"), "   \n\n ").expect("write");
        assert_eq!(tail(dir.path(), "demo.db", 4096), None, "and whitespace");
    }

    #[test]
    fn discarding_what_is_not_there_is_quiet() {
        let dir = tempfile::tempdir().expect("tempdir");
        discard(dir.path(), "never-ran");
        prepare(dir.path(), "demo.db").expect("prepared");
        discard(dir.path(), "demo.db");
        assert!(!path(dir.path(), "demo.db").exists());
    }
}
