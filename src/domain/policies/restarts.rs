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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_grows_and_stops_growing_at_five_minutes() {
        assert_eq!(wait_before(1), Duration::ZERO);
        let waits: Vec<_> = (1..=12).map(wait_before).collect();
        assert!(waits.windows(2).all(|pair| pair[0] <= pair[1]), "{waits:?}");
        assert_eq!(waits.last(), Some(&Duration::from_secs(300)));
        assert!(STABLE > wait_before(u32::MAX));
    }
}
