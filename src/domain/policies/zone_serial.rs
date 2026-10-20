/// The serial a zone takes after a change.
///
/// The clock where it is ahead, so a serial still reads as roughly when;
/// one more than before where it is not, because two changes in one second
/// and a clock that went back must both still move it forward — a name
/// server that sees the same serial keeps the zone it had.
pub fn next(previous: u32, now_secs: u64) -> u32 {
    let clock = u32::try_from(now_secs).unwrap_or(u32::MAX);
    clock.max(previous.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_serial_only_ever_moves_forward() {
        assert_eq!(next(0, 1_792_300_000), 1_792_300_000);
        assert_eq!(next(1_792_300_000, 1_792_300_000), 1_792_300_001);
        assert_eq!(next(1_792_300_005, 1_792_299_000), 1_792_300_006);
        assert_eq!(next(u32::MAX, 1), u32::MAX);
    }
}
