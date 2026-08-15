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
//! ## Every container, since a service's page asks the same question
//!
//! It was managed engines only, and the reason was the paragraph below:
//! nothing bounded the file, and a chatty web service writing to stdout
//! for a month is a disk leak with a slow fuse. But the log *page*
//! exists for every service — it is where somebody goes to find out why
//! one is not answering — and on a plain container it said the output
//! had not been kept and to deploy it again, which was advice that could
//! not work. Reported by Jorge, of an nginx.
//!
//! So the bound is here instead: [`trim`] keeps the end of a file that
//! has grown past [`MAX_BYTES`], and the pass that reconciles runs it.
//!
//! ## One run, not a history
//!
//! The file is truncated when the container starts. There is no
//! rotation here and no retention policy, so what somebody needs when a
//! container will not stay up is what *this* attempt said.
//!
//! **Following exists now** — [`read_from`], and the page and stream on
//! a service that use it. Searching and keeping still do not, and this
//! is not pretending otherwise: a run is what there is, and a
//! deployment is where it ends.
//!
//! One consequence, and it is visible on a node rather than in a test:
//! a container started **before** any of this shipped has no file at
//! all. That is not the same as a file with nothing in it, and the page
//! says so rather than reporting a quiet container — it was the
//! difference between "nothing to see" and "nobody was listening".

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

/// How large one container's log may get before its beginning is
/// dropped.
///
/// Big enough to hold what a service says between deployments, small
/// enough that thirty of them cannot fill a node — which is the number
/// that matters, because this is per container and a node runs many.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// What is kept when one is over: the end, because that is where the
/// reason a container stopped is.
const KEEP_BYTES: u64 = 2 * 1024 * 1024;

/// Drop the beginning of any log that has outgrown [`MAX_BYTES`].
///
/// Every file under `logs/`, not the ones a caller can name: a
/// container that is gone left its file behind, and the whole point of
/// the bound is the disk rather than any one service.
///
/// Returns how many were trimmed, so the pass that calls this can say
/// nothing on the ordinary tick and say something when it acted.
///
/// The file the shim is appending to is rewritten under it, which is
/// safe for the one thing this has to be safe for: the shim opened it
/// with `O_APPEND`, so its next write goes to the end of whatever is
/// there now. What can be lost is a line written during the rewrite,
/// and a log that loses a line while being trimmed is a better answer
/// than a node that fills its disk.
pub fn trim_all(data_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(data_dir.join("logs")) else {
        return 0;
    };
    let mut trimmed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() <= MAX_BYTES {
            continue;
        }
        match trim(&path, KEEP_BYTES) {
            Ok(()) => {
                trimmed += 1;
                tracing::info!(
                    file = %path.display(), was = metadata.len(),
                    "trimmed a container log"
                );
            }
            Err(error) => {
                tracing::warn!(file = %path.display(), %error, "could not trim a container log")
            }
        }
    }
    trimmed
}

/// Keep the last `keep` bytes of one file.
///
/// From the first newline inside the window, so what is left starts at a
/// line rather than half-way through one.
pub fn trim(path: &Path, keep: u64) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    if length <= keep {
        return Ok(());
    }
    file.seek(SeekFrom::Start(length - keep))?;
    let mut kept = Vec::new();
    file.read_to_end(&mut kept)?;
    let from = kept
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|at| at + 1)
        .unwrap_or(0);
    std::fs::write(path, &kept[from..])
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

/// How much of the end of a log a page opens on.
///
/// Enough to see why something is failing, small enough that the page
/// arrives. A container in a crash loop writes megabytes and the useful
/// part of a failure is the end of it — the same judgement as [`tail`],
/// with more room because this one is what somebody came to read.
pub const WINDOW: usize = 64 * 1024;

/// Where a reader is up to, and what to ask for next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    /// The byte to read from next time. Not `text.len()` added to the
    /// old one: a chunk that ended mid-character keeps the partial bytes
    /// unread rather than replacing them with `U+FFFD` for ever.
    pub next: u64,
    /// The file started again — a redeployment truncated it, so whatever
    /// the reader has on screen belongs to a container that is gone.
    pub restarted: bool,
}

/// Read from `offset` to the end.
///
/// The follower's half of this module. Returns `None` when there is no
/// file, which is an ordinary state and not a failure: a service that
/// has never run has never written one.
///
/// Two things it has to get right, and both were found by thinking about
/// what the file *is* rather than by reading:
///
/// - **The file is truncated on every deployment** — see the module
///   docs — so an offset from before one is past the end of a shorter
///   file. Reading from it would return nothing for ever while the
///   container was talking. That is `restarted`, and it starts over.
/// - **A read can land mid-character.** The shim appends bytes and this
///   can arrive between the two halves of a `ñ`. Splitting there and
///   lossily converting would put a replacement character into the
///   stream permanently, because the offset would have moved past it. So
///   an incomplete tail stays unread until the rest of it arrives.
pub fn read_from(data_dir: &Path, container_id: &str, offset: u64) -> Option<Chunk> {
    use std::io::{Read, Seek, SeekFrom};

    let path = path(data_dir, container_id);
    let mut file = std::fs::File::open(&path).ok()?;
    let length = file.metadata().ok()?.len();

    let (from, restarted) = match offset > length {
        true => (0, true),
        false => (offset, false),
    };
    // Opening in the middle of a long log is opening at its end. A page
    // that began at byte zero of a gigabyte would not arrive.
    let from = from.max(length.saturating_sub(WINDOW as u64));

    file.seek(SeekFrom::Start(from)).ok()?;
    let mut bytes = Vec::new();
    file.take((length - from) + 1)
        .read_to_end(&mut bytes)
        .ok()?;

    let complete = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        // Everything up to the first character that is not all here.
        Err(error) => error.valid_up_to(),
    };
    let text = String::from_utf8_lossy(&bytes[..complete]).into_owned();
    Some(Chunk {
        text,
        next: from + complete as u64,
        restarted,
    })
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

    /// Keeping every container's output is only safe with a bound on
    /// it: the shim appends and nothing rotates, so a chatty service
    /// running for a month is a node with a full disk. That is why this
    /// was managed engines only, and why the answer is the bound rather
    /// than the restriction — a log page that exists for every service
    /// and keeps nothing for most of them is a page that lies.
    #[test]
    fn a_log_that_outgrows_the_bound_keeps_its_end() {
        let dir = tempfile::tempdir().expect("tempdir");
        let logs = dir.path().join("logs");
        std::fs::create_dir_all(&logs).expect("mkdir");

        let small = logs.join("demo.quiet.log");
        std::fs::write(&small, b"one line\n").expect("write");

        // A megabyte of numbered lines, trimmed to the last few.
        let noisy = logs.join("demo.noisy.log");
        let mut written = String::new();
        for line in 0..40_000 {
            written.push_str(&format!("line {line}\n"));
        }
        std::fs::write(&noisy, &written).expect("write");
        let full = std::fs::metadata(&noisy).expect("stat").len();

        // Nothing is over the shipped bound, so the pass leaves both.
        assert_eq!(trim_all(dir.path()), 0);
        assert_eq!(std::fs::metadata(&noisy).expect("stat").len(), full);

        trim(&noisy, 1024).expect("trimmed");
        let kept = std::fs::read_to_string(&noisy).expect("read");
        assert!(kept.len() <= 1024, "{} bytes kept", kept.len());
        assert!(
            kept.ends_with("line 39999\n"),
            "the end is what a failure is at: {kept:.40}"
        );
        // And it starts at a line, not half-way through one.
        assert!(kept.starts_with("line "), "{kept:.40}");

        // The small one is untouched by either.
        trim(&small, 1024).expect("nothing to do");
        assert_eq!(std::fs::read_to_string(&small).expect("read"), "one line\n");
    }

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

    /// A reader that has seen everything is told there is nothing new,
    /// and its place does not move.
    #[test]
    fn following_returns_only_what_arrived_since() {
        let dir = tempfile::tempdir().expect("tempdir");
        prepare(dir.path(), "demo.web").expect("prepared");
        let log = path(dir.path(), "demo.web");

        std::fs::write(&log, "listening on 8080\n").expect("write");
        let first = read_from(dir.path(), "demo.web", 0).expect("read");
        assert_eq!(first.text, "listening on 8080\n");
        assert!(!first.restarted);

        let again = read_from(dir.path(), "demo.web", first.next).expect("read");
        assert_eq!(again.text, "", "it sent the same lines twice");
        assert_eq!(again.next, first.next);

        std::fs::write(&log, "listening on 8080\nGET /\n").expect("append");
        let next = read_from(dir.path(), "demo.web", first.next).expect("read");
        assert_eq!(next.text, "GET /\n");
    }

    /// A deployment truncates the file, so an offset from before one is
    /// past the end of a shorter file. Reading from it would return
    /// nothing for ever while the new container was talking — and
    /// whatever is on the reader's screen belongs to a container that no
    /// longer exists, which is why this is said rather than silently
    /// corrected.
    #[test]
    fn a_redeployment_is_noticed_rather_than_read_past() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = prepare(dir.path(), "demo.web").expect("prepared");
        std::fs::write(&log, "a long first run, several lines of it\n").expect("write");
        let seen = read_from(dir.path(), "demo.web", 0).expect("read").next;

        prepare(dir.path(), "demo.web").expect("again");
        std::fs::write(&log, "starting\n").expect("write");

        let after = read_from(dir.path(), "demo.web", seen).expect("read");
        assert!(
            after.restarted,
            "the reader was left waiting on a dead file"
        );
        assert_eq!(after.text, "starting\n");
    }

    /// The shim appends bytes and a read can land between the two halves
    /// of a `ñ`. Converting lossily there would put a replacement
    /// character into the stream *permanently*, because the offset moves
    /// past it and the real bytes are never read again.
    #[test]
    fn a_character_split_across_two_reads_survives_whole() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = prepare(dir.path(), "demo.web").expect("prepared");
        let whole = "conexión\n";
        let bytes = whole.as_bytes();

        // Cut inside the ó, which is two bytes.
        let split = whole.find('ó').expect("there") + 1;
        std::fs::write(&log, &bytes[..split]).expect("write");
        let first = read_from(dir.path(), "demo.web", 0).expect("read");
        assert_eq!(first.text, "conexi", "a partial character was emitted");

        std::fs::write(&log, bytes).expect("the rest");
        let second = read_from(dir.path(), "demo.web", first.next).expect("read");
        assert_eq!(
            format!("{}{}", first.text, second.text),
            whole,
            "the character did not survive the join"
        );
        assert!(!second.text.contains('\u{fffd}'), "{:?}", second.text);
    }

    /// Opening in the middle of a long log is opening at its end: a page
    /// that began at byte zero of a gigabyte would not arrive.
    #[test]
    fn a_long_log_is_opened_at_its_end() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = prepare(dir.path(), "demo.web").expect("prepared");
        let long = "x".repeat(WINDOW * 2);
        std::fs::write(&log, &long).expect("write");

        let chunk = read_from(dir.path(), "demo.web", 0).expect("read");
        assert_eq!(chunk.text.len(), WINDOW);
        assert_eq!(chunk.next, (WINDOW * 2) as u64, "and it is the *end*");
    }

    /// A service that has never run has never written one. An ordinary
    /// state, not a failure — the page says so rather than erroring.
    #[test]
    fn a_container_that_never_ran_has_no_log_and_that_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(read_from(dir.path(), "never.ran", 0).is_none());
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
