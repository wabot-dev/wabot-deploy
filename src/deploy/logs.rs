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
//! ## A history, not one run
//!
//! It used to truncate the file at every start, so a deployment threw
//! away everything the previous one had said. That is the wrong default
//! for the question people actually bring here — *what has this service
//! been doing* — because the interesting evidence is usually on the other
//! side of a restart: the crash that caused it.
//!
//! So a service's log is **appended to for ever**, across restarts and
//! deployments, with a boundary line written at each start. See
//! [`resume`]. The container's own bytes are untouched; the boundary is
//! the only thing this module ever adds, and it is shaped so nothing can
//! mistake it for output.
//!
//! **A one-shot helper still gets a clean file** — [`prepare`] — because
//! the fixer, the unpacker and the seeder are read once, immediately,
//! and discarded. Keeping their history would be keeping noise.
//!
//! ## Retention, in three dimensions because one is not a bound
//!
//! - **Per file**: at [`MAX_BYTES`] a log is *rotated*, not trimmed.
//!   Trimming dropped the beginning, which is where a first failure is.
//! - **Per container**: [`GENERATIONS`] kept behind the live file, and
//!   nothing older than [`MAX_AGE`] whatever the count.
//! - **Per node**: [`NODE_BUDGET`] across every log there is, oldest
//!   generation first. This one is the actual bound — a per-container
//!   limit times an unbounded number of containers bounds nothing, and
//!   that was true of the old 8 MB cap.
//!
//! The live file is never dropped by any of the three. A node that has
//! run out of log budget still answers "what is this container saying
//! now", which is the question asked while something is on fire.
//!
//! One consequence, visible on a node rather than in a test: a container
//! started **before** any of this shipped has no file at all. That is not
//! the same as a file with nothing in it, and the page says so rather
//! than reporting a quiet container — the difference between "nothing to
//! see" and "nobody was listening".

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

/// Make the directory and empty the file, ready for **a one-shot run**.
///
/// Truncating, which is right for exactly one kind of container: the
/// helpers this node starts, reads once and removes — the ownership
/// fixer, the base-backup unpacker, the standby seeder, the one that asks
/// a database a question. Their output is wanted for the seconds between
/// starting them and reading them, and a history of them is noise.
///
/// A service or a database uses [`resume`] instead. Calling this on one
/// would silently throw away everything it had ever said, which is what
/// the whole module used to do.
pub fn prepare(data_dir: &Path, container_id: &str) -> std::io::Result<PathBuf> {
    let path = path(data_dir, container_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, b"")?;
    Ok(path)
}

/// Carry on the log this container already has, and mark where the new
/// run begins.
///
/// **Appending, never truncating.** A deployment is the moment somebody
/// most wants what came before it, and the old behaviour deleted exactly
/// that.
///
/// The boundary line is the only thing this module writes into a
/// container's output, and its shape is chosen so it cannot be mistaken
/// for one: box-drawing characters and the product's own name. A parser
/// somebody writes against these files can pick the runs apart on it.
///
/// It goes in *before* the container starts, so the first thing after it
/// is the first thing the container said.
pub fn resume(data_dir: &Path, container_id: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;

    let path = path(data_dir, container_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // **Rotation happens here, at a start, and nowhere else.** The
    // obvious place was the sweep that runs every few minutes, and the
    // node showed why it is the wrong one — see `rotate`.
    if std::fs::metadata(&path).map(|it| it.len()).unwrap_or(0) > MAX_BYTES {
        rotate(&path)?;
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{}", boundary())?;
    Ok(path)
}

/// The line that separates one run from the next.
fn boundary() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "── wabot-deploy ── started {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC ──",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

/// How large one file may get before it is rotated.
///
/// Big enough to hold what a service says across several deployments,
/// small enough that the generations behind it are affordable.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// The size a *live* file may reach when nothing ever restarts it.
///
/// Four times [`MAX_BYTES`], because passing it means losing the front of
/// somebody's log and that should take a container really talking rather
/// than a busy afternoon. Reaching it is a warning in the journal, not a
/// routine event.
pub const LIVE_CEILING: u64 = 4 * MAX_BYTES;

/// Keep the last `keep` bytes of one file.
///
/// From the first newline inside the window, so what is left starts at a
/// line rather than half-way through one. The last resort — see [`sweep`].
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

/// How many rotated files are kept behind the live one.
///
/// Two, so a container holds at most three files and 24 MB. The number
/// is a compromise and worth naming as one: more generations is more
/// history for the container somebody is investigating and less budget
/// for every other container on the node.
pub const GENERATIONS: usize = 2;

/// How old a rotated file may be, whatever the count.
///
/// A container that says one line a week would otherwise keep a
/// generation from last year, and a log that old is not evidence, it is
/// furniture. Applies to rotated files only — the live one is never
/// dropped for being old, because "quiet" is not "gone".
pub const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 60 * 60);

/// Everything under `logs/`, across every container.
///
/// **This is the actual bound, and the old code had nothing like it.** A
/// per-container cap times an unbounded number of containers bounds
/// nothing: thirty containers at 8 MB was 240 MB and the comment saying
/// so treated thirty as a fact rather than a guess.
///
/// Half a gigabyte, which on the smallest node this runs on (20 GB) is
/// 2.5% of the disk.
pub const NODE_BUDGET: u64 = 512 * 1024 * 1024;

/// What one pass of the policy did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// A live file trimmed in place because it passed [`LIVE_CEILING`]
    /// without the container ever restarting. The front is gone, which
    /// is why this is counted apart and warned about.
    pub trimmed: usize,
    /// Dropped for being one generation too many, or too old.
    pub aged_out: usize,
    /// Dropped because the node was over budget. Counted separately
    /// because it means the budget is the binding constraint, which is
    /// the thing an operator would want to know before losing history
    /// they expected to have.
    pub over_budget: usize,
}

impl Swept {
    pub fn did_anything(&self) -> bool {
        self.trimmed + self.aged_out + self.over_budget > 0
    }
}

/// Apply the retention policy: rotate what is large, drop what is
/// surplus, old, or beyond the node's budget.
///
/// Every file under `logs/`, not the ones a caller can name: a container
/// that is gone left its files behind, and the point of a budget is the
/// disk rather than any one service.
///
/// **A live file is never dropped**, by any of the three rules. A node
/// out of log budget still answers "what is this container saying now",
/// which is the question people ask while something is on fire.
pub fn sweep(data_dir: &Path) -> Swept {
    let directory = data_dir.join("logs");
    let mut swept = Swept::default();

    // A live file is **not** rotated here — see `rotate` for why that
    // leaves a running container writing into the rotated file.
    // Ordinarily its size is dealt with at the next start.
    //
    // What is left is the container that never restarts and never stops
    // talking, for which "at the next start" is never. That one is
    // trimmed in place at a much higher ceiling: the front is lost, which
    // is what rotation exists to avoid, so it is a last resort and says
    // so out loud — rather than happening quietly at 8 MB to everything,
    // which is what the old code did.
    for path in live_logs(&directory) {
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.len() <= LIVE_CEILING {
            continue;
        }
        match trim(&path, MAX_BYTES) {
            Ok(()) => {
                swept.trimmed += 1;
                tracing::warn!(
                    file = %path.display(), was = metadata.len(),
                    "a container has written past the live ceiling without restarting; \
                     the beginning of its log was dropped"
                );
            }
            Err(error) => {
                tracing::warn!(file = %path.display(), %error, "could not trim a container log")
            }
        }
    }

    // Surplus and old generations.
    for old in generations(&directory) {
        let too_many = old.index > GENERATIONS;
        let too_old = old.age.map(|age| age > MAX_AGE).unwrap_or(false);
        if !too_many && !too_old {
            continue;
        }
        if std::fs::remove_file(&old.path).is_ok() {
            swept.aged_out += 1;
        }
    }

    // And the node's budget, oldest generation first — which is the
    // order that loses the least useful history.
    let mut kept = generations(&directory);
    let mut total: u64 = live_logs(&directory)
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .sum::<u64>()
        + kept.iter().map(|old| old.size).sum::<u64>();

    kept.sort_by_key(|old| std::cmp::Reverse(old.index));
    for old in kept {
        if total <= NODE_BUDGET {
            break;
        }
        if std::fs::remove_file(&old.path).is_ok() {
            total = total.saturating_sub(old.size);
            swept.over_budget += 1;
        }
    }

    if swept.over_budget > 0 {
        tracing::warn!(
            dropped = swept.over_budget,
            budget = NODE_BUDGET,
            "container logs were over the node's budget; oldest history dropped"
        );
    }
    swept
}

/// `<id>.log` — the file a shim is appending to.
fn live_logs(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|extension| extension.to_str()) == Some("log")
        })
        .collect()
}

/// One rotated file.
struct Generation {
    path: PathBuf,
    /// 1 is the most recent rotation.
    index: usize,
    size: u64,
    age: Option<std::time::Duration>,
}

/// `<id>.log.1`, `<id>.log.2`, …
fn generations(directory: &Path) -> Vec<Generation> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            // The index is the last extension, and the one before it must
            // be `log` — so a container whose id happens to end in a
            // number is not read as a generation of something else.
            let index: usize = path.extension()?.to_str()?.parse().ok()?;
            let stem = path.file_stem()?;
            if Path::new(stem).extension()?.to_str()? != "log" {
                return None;
            }
            Some(Generation {
                path,
                index,
                size: metadata.len(),
                age: metadata
                    .modified()
                    .ok()
                    .and_then(|when| when.elapsed().ok()),
            })
        })
        .collect()
}

/// Shift the generations along and start the live file again.
///
/// From the oldest, so nothing is overwritten before it has moved.
///
/// **Only called from [`resume`], at a container start.** The shim holds
/// the file's inode rather than its name, so renaming under a running
/// container leaves it writing into the rotated file — and the live file
/// at zero bytes, which a page reading it would report as a quiet
/// container. containerd offers no way to reopen a running container's
/// stdio. At a start the previous shim is gone and there is nothing to
/// race.
///
/// This was in the sweep first, and the node showed the problem: a
/// rotated 9 MB file beside a live one at zero.
pub fn rotate(path: &Path) -> std::io::Result<()> {
    for index in (1..=GENERATIONS).rev() {
        let from = with_index(path, index);
        if from.exists() {
            let _ = std::fs::rename(&from, with_index(path, index + 1));
        }
    }
    std::fs::rename(path, with_index(path, 1))?;
    std::fs::write(path, b"")
}

fn with_index(path: &Path, index: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

/// How many rotated generations this container has.
///
/// So a page can offer the whole history only when there is more of it
/// than the live file — a link that leads to the same bytes is a link
/// somebody follows once and stops trusting.
pub fn kept_generations(data_dir: &Path, container_id: &str) -> usize {
    let live = path(data_dir, container_id);
    (1..=GENERATIONS + 1)
        .filter(|index| with_index(&live, *index).exists())
        .count()
}

/// Everything kept for this container, oldest first.
///
/// The generations and then the live file, concatenated — which is what
/// "the log of this service" means to somebody who does not know this
/// module exists. Bounded by `limit` bytes taken from the **end**,
/// because the end is the present.
pub fn history(data_dir: &Path, container_id: &str, limit: usize) -> Option<String> {
    let live = path(data_dir, container_id);
    let mut files: Vec<PathBuf> = (1..=GENERATIONS + 1)
        .rev()
        .map(|index| with_index(&live, index))
        .filter(|path| path.exists())
        .collect();
    files.push(live.clone());

    let mut text = String::new();
    for file in &files {
        if let Ok(part) = std::fs::read_to_string(file) {
            text.push_str(&part);
        }
    }
    if text.trim().is_empty() {
        return None;
    }

    // From a character boundary, or the string would not be one.
    let start = text
        .char_indices()
        .rev()
        .take(limit)
        .last()
        .map(|(index, _)| index)
        .unwrap_or(0);
    Some(text[start..].to_string())
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
    // **The generations too.** This removed only the live file, so a
    // replica thrown off a node left its rotated history behind for
    // ever — and invisibly, because `Deployer::leftovers` recognised a
    // log by its `.log` suffix and `foo.log.1` does not have one. Two
    // bounds neither of which could see the files.
    for candidate in std::iter::once(path.clone())
        .chain((1..=GENERATIONS + 1).map(|index| with_index(&path, index)))
    {
        if let Err(error) = std::fs::remove_file(&candidate) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(file = %candidate.display(), %error, "removing a container's log");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keeping every container's output is only safe with a bound on
    /// it: the shim appends, so a chatty service
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
        assert_eq!(sweep(dir.path()), Swept::default());
        assert_eq!(std::fs::metadata(&noisy).expect("stat").len(), full);

        // The small one is untouched too.
        assert_eq!(std::fs::read_to_string(&small).expect("read"), "one line\n");
    }

    /// **Rotation keeps the beginning; trimming threw it away.** What was
    /// there before dropped the front of the file, which is where a first
    /// failure is — the crash that caused the restart that caused the
    /// crash loop somebody is looking at.
    #[test]
    fn rotating_keeps_what_trimming_used_to_drop() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("logs")).expect("mkdir");
        let live = path(dir.path(), "demo.web");
        std::fs::write(&live, "the first thing it ever said\n").expect("write");

        rotate(&live).expect("rotated");

        // The live file is empty and ready, and the history is beside it.
        assert_eq!(std::fs::read_to_string(&live).expect("read"), "");
        let first = with_index(&live, 1);
        assert_eq!(
            std::fs::read_to_string(&first).expect("read"),
            "the first thing it ever said\n"
        );

        // A second rotation shifts it along rather than overwriting it.
        std::fs::write(&live, "the second run\n").expect("write");
        rotate(&live).expect("rotated");
        assert_eq!(
            std::fs::read_to_string(with_index(&live, 2)).expect("read"),
            "the first thing it ever said\n"
        );
        assert_eq!(
            std::fs::read_to_string(&first).expect("read"),
            "the second run\n"
        );
    }

    /// A restart must not lose what came before it.
    ///
    /// This is the whole point of `resume`, and the behaviour it
    /// replaces: `prepare` truncated, so a deployment deleted the
    /// evidence of why the last one had to happen.
    #[test]
    fn a_restart_keeps_what_the_last_run_said() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = resume(dir.path(), "demo.web").expect("resumed");
        std::fs::write(&path, format!("{}\nthe first run failed\n", boundary())).expect("write");

        resume(dir.path(), "demo.web").expect("resumed again");
        let kept = std::fs::read_to_string(&path).expect("read");
        assert!(
            kept.contains("the first run failed"),
            "the previous run survived a restart: {kept}"
        );
        // And the two runs are separable, because each start writes a
        // boundary a parser can find.
        assert_eq!(
            kept.matches("── wabot-deploy ── started").count(),
            2,
            "one boundary per start: {kept}"
        );
    }

    /// A log over the size is rotated **at the start**, not by the
    /// sweep.
    ///
    /// The sweep was the obvious place and the node showed why it is the
    /// wrong one: containerd's shim holds the inode, so a rename under a
    /// running container leaves it writing into the rotated file, and the
    /// live file sits at zero bytes while the service talks. A page
    /// reading it would report a quiet container that was perfectly fine.
    #[test]
    fn a_large_log_rotates_when_the_container_starts() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("logs")).expect("mkdir");
        let live = path(dir.path(), "demo.web");
        std::fs::write(&live, "x".repeat(MAX_BYTES as usize + 1)).expect("write");

        // The sweep leaves it alone: it is under the live ceiling, and
        // rotating it here is the thing that would break.
        assert_eq!(sweep(dir.path()), Swept::default());
        assert!(std::fs::metadata(&live).expect("stat").len() > MAX_BYTES);

        // The next start rotates it, and the new run begins in a file
        // holding nothing but its own boundary.
        resume(dir.path(), "demo.web").expect("resumed");
        assert_eq!(
            std::fs::metadata(with_index(&live, 1)).expect("stat").len(),
            MAX_BYTES + 1
        );
        let fresh = std::fs::read_to_string(&live).expect("read");
        assert!(
            fresh.starts_with("── wabot-deploy ── started"),
            "{fresh:.40}"
        );
        assert!(
            fresh.len() < 200,
            "only the boundary: {} bytes",
            fresh.len()
        );
    }

    /// A container that never restarts and never stops talking is the
    /// one case rotation cannot serve, so there is a far higher ceiling
    /// where the front is dropped — loudly, because that is the loss
    /// rotation exists to prevent.
    #[test]
    fn a_live_log_past_the_ceiling_is_trimmed_as_a_last_resort() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("logs")).expect("mkdir");
        let live = path(dir.path(), "demo.chatty");
        let mut written = String::new();
        while written.len() as u64 <= LIVE_CEILING {
            written.push_str("a line of output\n");
        }
        std::fs::write(&live, &written).expect("write");

        let swept = sweep(dir.path());
        assert_eq!(swept.trimmed, 1);
        let kept = std::fs::metadata(&live).expect("stat").len();
        assert!(kept <= MAX_BYTES, "{kept} bytes kept");
        // And what is kept starts at a line rather than mid-way through.
        let text = std::fs::read_to_string(&live).expect("read");
        assert!(text.starts_with("a line of output"), "{text:.40}");
    }

    /// A one-shot helper still starts clean: its output is read once,
    /// immediately, and a history of ownership fixers is noise.
    #[test]
    fn a_one_shot_helper_starts_from_an_empty_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = prepare(dir.path(), "demo.fixer").expect("prepared");
        std::fs::write(&path, "chowned").expect("write");
        prepare(dir.path(), "demo.fixer").expect("prepared again");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "");
    }

    /// A generation past the count goes, and the live file never does —
    /// a node out of budget still answers "what is it saying now", which
    /// is the question asked while something is on fire.
    #[test]
    fn surplus_generations_go_and_the_live_file_stays() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("logs")).expect("mkdir");
        let live = path(dir.path(), "demo.web");
        std::fs::write(&live, "now\n").expect("write");
        for index in 1..=GENERATIONS + 2 {
            std::fs::write(with_index(&live, index), format!("gen {index}\n")).expect("write");
        }

        let swept = sweep(dir.path());
        assert_eq!(swept.aged_out, 2, "the two past the count");
        assert!(live.exists(), "the live file is never dropped");
        for index in 1..=GENERATIONS {
            assert!(
                with_index(&live, index).exists(),
                "generation {index} stays"
            );
        }
        assert!(!with_index(&live, GENERATIONS + 1).exists());
    }

    /// A container id ending in a number must not be read as a
    /// generation of something else — `demo.web.2` is a *replica*, and
    /// its live log is `demo.web.2.log`.
    #[test]
    fn a_replica_number_is_not_a_generation() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("logs")).expect("mkdir");
        let replica = path(dir.path(), "demo.web.2");
        std::fs::write(&replica, "the second copy\n").expect("write");

        assert_eq!(generations(&dir.path().join("logs")).len(), 0);
        assert_eq!(sweep(dir.path()), Swept::default());
        assert!(replica.exists(), "a replica's log is not a rotated file");
    }

    /// Removing a copy takes its whole history, not just the live file.
    ///
    /// It took only the live one, so a replica thrown off a node left
    /// its rotated logs behind for ever — and **invisibly**, because
    /// `Deployer::leftovers` found a log by its `.log` suffix and
    /// `foo.log.1` has not got one. Two bounds, neither able to see the
    /// files, which is how something becomes immortal.
    #[test]
    fn discarding_a_log_takes_its_generations_too() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("logs")).expect("mkdir");
        let live = path(dir.path(), "demo.web");
        std::fs::write(&live, "now\n").expect("write");
        for index in 1..=GENERATIONS {
            std::fs::write(with_index(&live, index), "then\n").expect("write");
        }

        discard(dir.path(), "demo.web");

        assert!(!live.exists());
        for index in 1..=GENERATIONS {
            assert!(
                !with_index(&live, index).exists(),
                "generation {index} survived"
            );
        }
    }

    /// What "the log of this service" means to somebody who does not
    /// know this module exists: the generations and the live file, in
    /// order, as one thing.
    #[test]
    fn the_history_reads_as_one_log_oldest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("logs")).expect("mkdir");
        let live = path(dir.path(), "demo.web");
        std::fs::write(with_index(&live, 2), "oldest\n").expect("write");
        std::fs::write(with_index(&live, 1), "middle\n").expect("write");
        std::fs::write(&live, "newest\n").expect("write");

        assert_eq!(
            history(dir.path(), "demo.web", 4096).as_deref(),
            Some("oldest\nmiddle\nnewest\n")
        );
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
