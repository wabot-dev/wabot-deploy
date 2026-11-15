//! The one place that reads the wall clock.
//!
//! Milliseconds since the epoch, which is what every timestamp column in
//! this database holds.

use std::sync::atomic::{AtomicI64, Ordering};

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// [`now_ms`], never the same twice in this process.
///
/// For two moments whose order is the whole question — a deployment asked
/// for and a stop — where the same millisecond would say neither came
/// first.
pub(crate) fn distinct_now_ms() -> i64 {
    static LAST: AtomicI64 = AtomicI64::new(0);
    let now = now_ms();
    let previous = LAST
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
            Some(now.max(last + 1))
        })
        .unwrap_or(now);
    now.max(previous + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_distinct_moments_in_the_same_millisecond_still_come_one_after_the_other() {
        let moments: Vec<i64> = (0..1_000).map(|_| distinct_now_ms()).collect();

        assert!(moments.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
