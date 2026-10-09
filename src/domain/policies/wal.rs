//! What a WAL file is called, and which ones may go.
//!
//! Pure arithmetic over names: a segment is `{timeline}{high}{low}` in
//! hexadecimal, and an LSN is two 32-bit halves with a slash between
//! them. Nothing here touches a disk.

/// How many bytes of write-ahead log one segment holds.
///
/// Postgres's default, and not read from the server: this node never
/// passes `--wal-segsize`, so `initdb` uses the default and every
/// database here has it. A node that met a database built elsewhere
/// with a different size would compute the wrong name — which is why
/// this is a constant with a reason rather than a magic number.
pub const SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
/// Read a `Start-LSN` as Postgres writes it: two hex halves either side
/// of a slash, `0/13000060`.
pub fn lsn(text: &str) -> Option<u64> {
    let (high, low) = text.trim().split_once('/')?;
    let high = u64::from_str_radix(high.trim(), 16).ok()?;
    let low = u64::from_str_radix(low.trim(), 16).ok()?;
    Some((high << 32) | low)
}
/// The segment file that holds this position.
///
/// The name is three fixed-width hex fields: the timeline, then the
/// segment number split into its high and low halves. The split is at
/// however many segments fit in a 4 GB "log id" — 256 of them at the
/// default size — which is the part that looks arbitrary and is not: it
/// is why the low field never exceeds `FF` on a default install, and why
/// somebody reading an archive sees the third field roll over rather
/// than count on for ever.
pub fn segment_name(timeline: u32, lsn: u64) -> String {
    let segment = lsn / SEGMENT_BYTES;
    let per_id = (1u64 << 32) / SEGMENT_BYTES;
    format!(
        "{timeline:08X}{:08X}{:08X}",
        segment / per_id,
        segment % per_id
    )
}
/// Whether this file may be deleted, given the oldest one still needed.
///
/// A plain comparison, because the names are built for it. Anything that
/// is not a segment name — the archive's own `.tmp` leftovers, a file
/// somebody dropped in there — is **kept**, which is the safe direction:
/// this deletes the ability to restore, so what it does not understand
/// it does not touch.
pub fn may_delete(file: &str, oldest_needed: &str) -> bool {
    let name = file.strip_suffix(".gz").unwrap_or(file);
    // 24 hex digits and nothing else. A backup history file
    // (`…​.00000028.backup`) fails this and is kept, which is right:
    // they are small and they are what somebody reads to work out what
    // happened.
    if name.len() != 24 || !name.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    name < oldest_needed
}
/// The start position a `pg_basebackup` manifest records.
///
/// Its own JSON, version 2, and this reads two fields of it. Parsed
/// rather than assumed because it is the number the whole rule rests
/// on: get it wrong in the safe direction and the archive grows, get it
/// wrong in the other and a restore stops at a hole.
///
/// `None` when the manifest cannot be read or does not say. The caller
/// treats that as "this backup needs everything", which is the only
/// answer that cannot destroy a window.
pub fn start_of(manifest: &str) -> Option<(u32, u64)> {
    let value: serde_json::Value = serde_json::from_str(manifest).ok()?;
    let range = value.get("WAL-Ranges")?.as_array()?.first()?;
    let timeline = range.get("Timeline")?.as_u64()? as u32;
    let start = lsn(range.get("Start-LSN")?.as_str()?)?;
    Some((timeline, start))
}
/// Read a segment name back into the two numbers it is made of.
///
/// The inverse of [`segment_name`], and it exists for one reason: to
/// tell whether the archive is *continuous*. A window with a hole in it
/// is not a shorter window, it is two windows with the second
/// unreachable — so the only way to promise "any moment between here and
/// there" is to check that every name between here and there is present.
pub fn segment_of(name: &str) -> Option<(u32, u64)> {
    if name.len() != 24 {
        return None;
    }
    let timeline = u32::from_str_radix(&name[0..8], 16).ok()?;
    let high = u64::from_str_radix(&name[8..16], 16).ok()?;
    let low = u64::from_str_radix(&name[16..24], 16).ok()?;
    let per_id = (1u64 << 32) / SEGMENT_BYTES;
    Some((timeline, high * per_id + low))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a manifest the node actually wrote, so the shape is
    /// Postgres's rather than mine.
    #[test]
    fn a_manifest_says_where_its_backup_begins() {
        let manifest = r#"{ "PostgreSQL-Backup-Manifest-Version": 2,
            "System-Identifier": 7673202918516576284,
            "Files": [{ "Path": "backup_label", "Size": 227 }],
            "WAL-Ranges": [{ "Timeline": 1, "Start-LSN": "0/13000060",
                             "End-LSN": "0/13000168" }],
            "Manifest-Checksum": "abc" }"#;

        assert_eq!(start_of(manifest), Some((1, 0x13000060)));

        // Unreadable is not "needs nothing" — the caller reads `None`
        // as "needs everything", which is the only answer that cannot
        // put a hole in somebody's recovery window.
        assert_eq!(start_of("{}"), None);
        assert_eq!(start_of("not json"), None);
    }
    /// The three fields are a timeline and a segment number split at
    /// however many fit in a 4 GB log id.
    #[test]
    fn a_position_names_the_segment_that_holds_it() {
        // The real one from the node: 0/13000060 is segment 0x13 of
        // log 0, on timeline 1.
        assert_eq!(segment_name(1, 0x13000060), "000000010000000000000013");
        // The first segment of all.
        assert_eq!(segment_name(1, 0), "000000010000000000000000");
        // And the roll-over, which is the part of the name that looks
        // arbitrary: 256 segments of 16 MB fill one log id.
        assert_eq!(segment_name(1, 0xFF000000), "0000000100000000000000FF");
        assert_eq!(segment_name(1, 0x100000000), "000000010000000100000000");
        assert_eq!(segment_name(2, 0x100000000), "000000020000000100000000");
    }
    /// Older goes, newer stays, and anything this does not understand
    /// stays — because what it deletes is the ability to restore.
    #[test]
    fn only_what_no_kept_backup_needs_is_deletable() {
        let oldest = "000000010000000000000013";

        assert!(may_delete("000000010000000000000012.gz", oldest));
        assert!(may_delete("000000010000000000000001.gz", oldest));
        // The one the backup starts in is needed, not spare.
        assert!(!may_delete("000000010000000000000013.gz", oldest));
        assert!(!may_delete("000000010000000000000014.gz", oldest));

        // Not a segment name: kept, whatever it is.
        assert!(!may_delete(
            "000000010000000000000012.00000028.backup",
            oldest
        ));
        assert!(!may_delete("000000010000000000000012.gz.tmp", oldest));
        assert!(!may_delete("notes.txt", oldest));
        assert!(!may_delete("", oldest));

        // A later timeline sorts after an earlier one, which is what
        // the leading field is for — a promoted database's log must not
        // be deleted by a rule about the log it branched from.
        assert!(!may_delete("000000020000000000000001.gz", oldest));
    }
    /// A name reads back into the numbers it was built from, which is
    /// what makes continuity checkable.
    #[test]
    fn a_segment_name_reads_back() {
        for (timeline, lsn) in [
            (1u32, 0u64),
            (1, 0x13000060),
            (1, 0x100000000),
            (2, 0xFF000000),
        ] {
            let name = segment_name(timeline, lsn);
            let (read_timeline, segment) = segment_of(&name).expect("read back");
            assert_eq!(read_timeline, timeline, "{name}");
            assert_eq!(segment, lsn / SEGMENT_BYTES, "{name}");
        }
        assert_eq!(segment_of("short"), None);
        assert_eq!(segment_of("zzzzzzzzzzzzzzzzzzzzzzzz"), None);
    }
    /// Two halves of hex, and a slash. Postgres writes it this way
    /// everywhere a position appears.
    #[test]
    fn a_position_reads_as_postgres_writes_it() {
        assert_eq!(lsn("0/13000060"), Some(0x13000060));
        assert_eq!(lsn("1/0"), Some(0x1_00000000));
        assert_eq!(lsn(" 0/D000028 "), Some(0xD000028));
        assert_eq!(lsn("nonsense"), None);
        assert_eq!(lsn("0/"), None);
    }
}
