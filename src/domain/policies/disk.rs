//! When a filesystem is too full to leave alone.

/// The share of the disk below which a node says so.
///
/// A share rather than a size, because what it buys is time and time
/// scales with the disk: on node-0's 15 GB it is about five hours of a
/// database not archiving its log, at the 300 MB an hour measured there
/// on 2026-11-02.
pub const FREE_FLOOR_PERCENT: u64 = 10;

pub fn nearly_full(total: u64, available: u64) -> bool {
    total > 0 && available.saturating_mul(100) < total.saturating_mul(FREE_FLOOR_PERCENT)
}

#[cfg(test)]
mod tests {
    use super::nearly_full;

    #[test]
    fn a_disk_is_nearly_full_below_a_tenth_free_and_never_when_unread() {
        assert!(nearly_full(15_000, 1_400));
        assert!(!nearly_full(15_000, 1_500));
        assert!(!nearly_full(15_000, 8_000));
        assert!(
            !nearly_full(0, 0),
            "a filesystem that could not be read is not full"
        );
    }
}
