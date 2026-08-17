//! `wabot-deploy log-writer` — the program containerd hands a
//! container's output to.
//!
//! Not for people. containerd's shim starts this when a container using
//! `binary://` logging starts, one process per container, and it lives as
//! long as that container does.
//!
//! ## The protocol, measured rather than recalled
//!
//! - `CONTAINER_ID` and `CONTAINER_NAMESPACE` in the environment.
//! - **fd 3** is the container's stdout, **fd 4** its stderr.
//! - **fd 5 is a readiness pipe, and containerd blocks until it closes.**
//!   A logger that reads only 3 and 4 hangs container creation for ever.
//!   That is not in any error message; it is a process that never
//!   returns, and it cost one wasted node run to find.
//!
//! ## The rule that makes this safe to exist at all
//!
//! **It must never stop reading.** `file://` was chosen for this
//! platform's logs precisely because it needs no reader: a pipe nobody
//! drains fills up, and the container's next write blocks on it — the
//! service hangs, not the log.
//!
//! A binary logger reintroduces that hazard by being a reader, so every
//! failure here drains and discards rather than stopping. A full disk
//! loses lines; it does not hang a service. That trade is the whole
//! reason this is allowed to run beside somebody's production container,
//! and it is why there is no `?` in the loop below.
//!
//! ## What it adds to each line
//!
//! `2026-08-17T20:03:18Z stdout ` — an RFC 3339 instant in UTC, then
//! which stream it came from. 28 bytes, against a mean line of 148 on a
//! real nginx, so about a fifth more bytes. The console says so on the
//! switch that turns this on, because the cost is invisible until
//! somebody looks for history that is no longer there.
//!
//! Two streams into one file, in arrival order, because that is the order
//! things happened in and interleaving is what makes a log readable. The
//! `stdout`/`stderr` word is what lets them be separated again — which
//! `file://` cannot offer at all, since it merges them with no marker.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc;

/// Where the container's stdout arrives.
const STDOUT_FD: i32 = 3;
/// Where its stderr arrives.
const STDERR_FD: i32 = 4;
/// The pipe containerd waits on. Closing it says "I am reading now".
const READY_FD: i32 = 5;

pub fn run(path: PathBuf) -> anyhow::Result<i32> {
    // **Before anything else.** containerd is blocked until this closes,
    // and everything below — opening a file, making a directory — can
    // take a moment or fail. A logger that signalled readiness only after
    // succeeding would turn a full disk into a container that never
    // starts.
    close_ready();

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // One channel, two readers, one writer. The order lines arrive in is
    // the order they happened in, and a single writer is what keeps a
    // line whole — two threads appending to one file interleave inside
    // lines, which is a log nobody can read.
    let (sender, receiver) = mpsc::channel::<String>();
    for (fd, stream) in [(STDOUT_FD, "stdout"), (STDERR_FD, "stderr")] {
        let sender = sender.clone();
        std::thread::spawn(move || drain(fd, stream, sender));
    }
    // The originals, or the channel never closes and this never returns.
    drop(sender);

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();

    // No `?` anywhere in here. See the module docs: the one thing this
    // must not do is stop reading, because the pipe filling up blocks the
    // container.
    for line in receiver {
        let Some(handle) = file.as_mut() else {
            continue;
        };
        if handle.write_all(line.as_bytes()).is_err() {
            // Dropped, and dropped for good rather than retried in a
            // loop: retrying a write to a full disk is how a logger
            // stops draining.
            file = None;
        }
    }
    Ok(0)
}

/// Read one stream to its end, stamping each line.
///
/// Lossily decoded on purpose. A container may write any bytes at all,
/// and this is the one place where refusing invalid UTF-8 would mean
/// abandoning the stream — which is the failure this must never have.
fn drain(fd: i32, stream: &'static str, sender: mpsc::Sender<String>) {
    use std::os::fd::FromRawFd;

    // SAFETY: containerd opened these before exec and nothing else in
    // this process touches them. One owner each, taken once.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let reader = BufReader::new(file);

    for line in reader.split(b'\n') {
        let Ok(bytes) = line else {
            // A read error is the end of this stream, not of the other
            // one, and not of the process.
            return;
        };
        let text = String::from_utf8_lossy(&bytes);
        // The line as it was, with the prefix. A trailing `\r` from a
        // program writing CRLF is left alone: this transcribes rather
        // than tidies.
        if sender.send(format!("{} {stream} {text}\n", now())).is_err() {
            return;
        }
    }
}

/// Now, as RFC 3339 in UTC to the second.
///
/// To the second and not finer: a millisecond costs four more bytes on
/// every line, and nobody reading a container log is ordering events
/// inside one second — the ordering is already the order they arrive in.
fn now() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

/// Tell containerd it may start the container.
fn close_ready() {
    // SAFETY: closing a descriptor this process owns and never uses
    // again. containerd is blocked on the other end until it goes.
    unsafe {
        libc::close(READY_FD);
    }
}

/// How many bytes the prefix adds to every line.
///
/// `2026-08-17T20:03:18Z stderr ` — the widest of the two stream words,
/// because the console quotes this as a cost and a cost should not be
/// quoted at its best case.
///
/// Read only by the tests, and that is its job: the console states the
/// figure as prose, in a sentence that has to stay a fixed translation
/// key, so this is what a test compares against both the format written
/// here and the number written there.
#[cfg_attr(not(test), allow(dead_code))]
pub const PREFIX_BYTES: usize = 28;

#[cfg(test)]
mod tests {
    use super::*;

    /// The number the console shows as the disk cost has to be the number
    /// this actually writes. A form quoting a cost it does not impose is
    /// worse than one quoting none, because somebody plans around it.
    #[test]
    fn the_prefix_is_the_width_the_console_quotes() {
        let line = format!("{} {} ", now(), "stderr");
        assert_eq!(line.len(), PREFIX_BYTES, "{line:?}");

        // And `stdout` is the same width, which is a small mercy: both
        // stream words are six letters, so every line carries the same
        // prefix and the figure quoted is the figure, not a range. The
        // first version of this asserted stdout was one narrower and was
        // simply wrong about the alphabet.
        let other = format!("{} {} ", now(), "stdout");
        assert_eq!(other.len(), PREFIX_BYTES);
    }

    /// And the console must quote the same number.
    ///
    /// The form asks somebody to accept a cost, so the figure on it has
    /// to be the one this imposes. It cannot be interpolated — the
    /// sentence is a translation key and has to stay fixed — so the tie
    /// is here, reading the source the way the translation scan does.
    ///
    /// If this fails, change the sentence and its Spanish rather than
    /// deleting the test: a form quoting a cost it does not impose is
    /// worse than one quoting none, because somebody plans around it.
    #[test]
    fn the_console_quotes_the_same_number_it_writes() {
        let page = include_str!("../console/services.rs");
        assert!(
            page.contains(&format!("{PREFIX_BYTES} bytes on every line")),
            "the logs card no longer states {PREFIX_BYTES} bytes"
        );
    }

    /// The instant is sortable text, which is what makes `sort` and
    /// `grep` on a log file work at all.
    #[test]
    fn the_instant_is_rfc_3339_in_utc() {
        let stamp = now();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'), "{stamp}");
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[10..11], "T");
    }
}
