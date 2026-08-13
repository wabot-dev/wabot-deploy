//! How much memory a service may have, chosen from a list.
//!
//! ## A list, not a field
//!
//! A free-text number is a question most operators cannot answer, and
//! the ones who can would rather not: what matters is not "how many
//! bytes" but "which of the sizes this node can afford". A ladder also
//! makes the second half possible — a database's own arithmetic is
//! derived from the size, and a rung is something a table of settings
//! can be written against.
//!
//! ## The sizes are powers of two, and the labels are not
//!
//! 64 MB here is 64 MiB, like everywhere else in this trade. Saying
//! `67.1 MB` would be accurate and would match nothing an operator has
//! ever read on a VPS invoice.

/// The rungs, in bytes. Smallest first, which is the order the console
/// offers them in.
///
/// It stops at 4 GB because the node this runs on has 1, and a list
/// whose last entries are impossible on the machine reading it is a
/// list that teaches the operator to ignore it.
pub const LADDER: [u64; 7] = [
    64 * MB,
    128 * MB,
    256 * MB,
    512 * MB,
    1024 * MB,
    2048 * MB,
    4096 * MB,
];

const MB: u64 = 1024 * 1024;

/// The smallest rung. Enough for Postgres with `shared_buffers` at 16
/// MB, and not enough for much else — which is the honest bottom of
/// the ladder rather than a number that makes the list look generous.
pub const SMALLEST: u64 = LADDER[0];

/// A rung by the number in a form, or `None` for "no ceiling".
///
/// Refuses anything that is not on the ladder, rather than rounding to
/// the nearest. A form that quietly gives 128 MB to somebody who asked
/// for 100 is one whose page then disagrees with the container.
pub fn parse(text: &str) -> Result<Option<u64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let bytes: u64 = text
        .parse()
        .map_err(|_| format!("{text:?} is not one of the sizes on offer"))?;
    match LADDER.contains(&bytes) {
        true => Ok(Some(bytes)),
        false => Err(format!("{text:?} is not one of the sizes on offer")),
    }
}

/// What a rung is called on a page.
///
/// Not translated. `MB` and `GB` read the same in both languages, and
/// a size is a quantity rather than prose.
pub fn label(bytes: u64) -> String {
    match bytes >= 1024 * MB {
        true => format!("{} GB", bytes / (1024 * MB)),
        false => format!("{} MB", bytes / MB),
    }
}

/// How big `/dev/shm` should be for a container with this ceiling.
///
/// A quarter, which is the share Postgres's own documentation assumes
/// when it talks about shared memory, bounded at both ends:
///
/// * **16 MB at the bottom.** Below that a parallel worker cannot get
///   a segment at all, and the failure reads as a query error rather
///   than as a sizing problem.
/// * **512 MB at the top.** A tmpfs page is charged to the cgroup that
///   wrote it, so the limit already bounds this — the cap is there so a
///   4 GB database cannot fill two of them with shared memory before
///   anything notices.
///
/// A container with no ceiling keeps `spec::DEFAULT_SHM`, which is
/// Docker's 64 MB. The caller decides that by passing `None`.
pub fn shm_for(memory_limit: u64) -> u64 {
    (memory_limit / 4).clamp(16 * MB, 512 * MB)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A form that rounded 100 MB up to 128 would show one number and
    /// run another.
    #[test]
    fn a_size_that_is_not_on_the_ladder_is_refused_rather_than_rounded() {
        assert_eq!(parse("").expect("empty is no ceiling"), None);
        assert_eq!(parse(" ").expect("blank is no ceiling"), None);
        assert_eq!(parse("67108864").expect("64 MB"), Some(64 * MB));

        for wrong in ["100", "0", "64", "64MB", "nonsense", "-1"] {
            assert!(parse(wrong).is_err(), "{wrong} was accepted");
        }
    }

    #[test]
    fn the_labels_read_the_way_an_invoice_does() {
        assert_eq!(label(64 * MB), "64 MB");
        assert_eq!(label(512 * MB), "512 MB");
        assert_eq!(label(1024 * MB), "1 GB");
        assert_eq!(label(4096 * MB), "4 GB");
    }

    /// Every rung has to have a label somebody would recognise, which
    /// is the whole reason the ladder is powers of two.
    #[test]
    fn every_rung_is_labelled_and_parses_back() {
        for rung in LADDER {
            let label = label(rung);
            assert!(label.ends_with(" MB") || label.ends_with(" GB"), "{label}");
            assert_eq!(parse(&rung.to_string()).expect("parses"), Some(rung));
        }
    }

    /// 64 MB of shared memory in a 64 MB container is the whole
    /// ceiling spent on one tmpfs, and 16 MB in a 4 GB one is where
    /// parallel query starts failing.
    #[test]
    fn shared_memory_follows_the_ceiling_within_bounds() {
        assert_eq!(shm_for(64 * MB), 16 * MB, "a quarter, at the floor");
        assert_eq!(shm_for(256 * MB), 64 * MB);
        assert_eq!(shm_for(2048 * MB), 512 * MB);
        assert_eq!(shm_for(4096 * MB), 512 * MB, "capped");

        for rung in LADDER {
            assert!(shm_for(rung) <= rung, "{} would be the whole of it", rung);
        }
    }
}
