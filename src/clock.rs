//! The one place that reads the wall clock.
//!
//! Milliseconds since the epoch, which is what every timestamp column in
//! this database holds.

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
