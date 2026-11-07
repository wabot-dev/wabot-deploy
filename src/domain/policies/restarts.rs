//! How long a copy that keeps stopping waits before it is started again.

use std::time::Duration;

/// Running this long without stopping forgets every stop before it.
pub const STABLE: Duration = Duration::from_secs(600);

/// The wait before the `attempt`th start of one episode of stopping.
///
/// The first at once, then growing to five minutes and no further: often
/// enough that a copy comes back soon after its cause goes, rarely enough
/// that a copy which cannot stay up is not the load on the node.
pub fn wait_before(attempt: u32) -> Duration {
    Duration::from_secs(match attempt {
        0 | 1 => 0,
        2 => 10,
        3 => 30,
        4 => 60,
        5 => 120,
        6 => 240,
        _ => 300,
    })
}

/// What a copy's row says once this node has started it again.
///
/// Written where a failure is, because it travels where a failure does —
/// on the report to the node that placed the copy — and read back by
/// [`is_note`], so the sentence and the test for it live together.
pub fn note(when: &str, attempt: u32, how: &str, last_line: Option<&str>) -> String {
    match last_line {
        Some(line) => {
            format!("{NOTE_OPENS}{when} (attempt {attempt}) because {how}; its last line: {line}")
        }
        None => format!("{NOTE_OPENS}{when} (attempt {attempt}) because {how}"),
    }
}

/// Whether a copy's reason is that note rather than a failure.
pub fn is_note(reason: &str) -> bool {
    reason.starts_with(NOTE_OPENS)
}

const NOTE_OPENS: &str = "started again at ";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_note_a_restart_leaves_is_told_apart_from_a_failure() {
        assert!(is_note(&note(
            "05:38 UTC",
            17,
            "it stopped with exit 1",
            Some("Node.js v22.23.3")
        )));
        assert!(is_note(&note("05:49 UTC", 1, "it was not running", None)));
        assert!(!is_note(
            "could not pull docker.io/library/ghost:6.71.0-alpine"
        ));
        assert!(!is_note(""));
    }

    #[test]
    fn the_wait_grows_and_stops_growing_at_five_minutes() {
        assert_eq!(wait_before(1), Duration::ZERO);
        let waits: Vec<_> = (1..=12).map(wait_before).collect();
        assert!(waits.windows(2).all(|pair| pair[0] <= pair[1]), "{waits:?}");
        assert_eq!(waits.last(), Some(&Duration::from_secs(300)));
        assert!(STABLE > wait_before(u32::MAX));
    }
}
