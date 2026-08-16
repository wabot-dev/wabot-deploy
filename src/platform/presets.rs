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

/// What a node keeps for itself, and never promises to a container.
///
/// The console, the edge, the deploy path, containerd and its shims all
/// live outside anybody's cgroup, and a node that promised its last
/// megabyte would be a node whose own console dies first — which is also
/// the page somebody would go to in order to undo it.
///
/// A fraction *and* a floor, because neither alone survives both ends of
/// the range this runs on: 15 % of the 1 GB test node is 154 MB, which
/// is under what this process plus containerd actually use, and a flat
/// 256 MB on a 32 GB machine would reserve almost nothing worth
/// reserving.
pub const NODE_RESERVE_FRACTION: u64 = 15;
pub const NODE_RESERVE_FLOOR: u64 = 256 * MB;

/// And the same for CPU, in millicores. A quarter core, flat: the
/// node's own work is answering a console and running a reconcile loop,
/// and neither grows with the size of the machine.
pub const NODE_RESERVE_MILLICORES: u32 = 250;

/// How much of a machine may be promised to containers.
pub fn allocatable_memory(total: u64) -> u64 {
    let reserve = (total * NODE_RESERVE_FRACTION / 100).max(NODE_RESERVE_FLOOR);
    total.saturating_sub(reserve)
}

/// The same for CPU. `total` is the machine's cores in millicores.
pub fn allocatable_cpu(total: u32) -> u32 {
    total.saturating_sub(NODE_RESERVE_MILLICORES)
}

/// The CPU rungs, in millicores. A thousand is one core.
///
/// A ladder rather than a free number, for the reason the memory one is:
/// a field that takes anything is a field somebody puts 50 in, and 50
/// millicores is a container that cannot finish starting. It starts at a
/// quarter core because that is roughly what a small web service idles
/// at plus room to answer, and stops at four for the reason the memory
/// ladder stops at 4 GB — a list whose last entries are impossible on
/// the machine reading it teaches the operator to ignore it.
///
/// Not tied to the memory ladder. They are different resources with
/// different failure modes: running out of memory kills a container,
/// running out of CPU makes it slow, and a service can perfectly well
/// want a lot of one and little of the other.
pub const CPU_LADDER: [u32; 6] = [250, 500, 1000, 2000, 3000, 4000];

/// What a CPU rung is called on a page.
///
/// Cores below a thousand read as fractions because that is how somebody
/// thinks about them — "half a core" and not "500 millicores" — and at
/// or above one core as cores, for the same reason.
pub fn cpu_label(millicores: u32) -> String {
    match millicores {
        0 => "no ceiling".to_string(),
        250 => "¼ core".to_string(),
        500 => "½ core".to_string(),
        millicores if millicores % 1000 == 0 => match millicores / 1000 {
            1 => "1 core".to_string(),
            cores => format!("{cores} cores"),
        },
        millicores => format!("{millicores}m"),
    }
}

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

/// The same, for a CPU rung. Empty is "no ceiling".
pub fn parse_cpu(text: &str) -> Result<Option<u32>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let millicores: u32 = text
        .parse()
        .map_err(|_| format!("{text:?} is not one of the CPU sizes on offer"))?;
    match CPU_LADDER.contains(&millicores) {
        true => Ok(Some(millicores)),
        false => Err(format!("{text:?} is not one of the CPU sizes on offer")),
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
    /// A node keeps enough for itself at both ends of the range it runs
    /// on, which is why the reserve is a fraction **and** a floor.
    ///
    /// Neither alone survives both: 15 % of the 1 GB test node is
    /// 154 MB, under what this process and containerd actually use, so
    /// a fraction alone would promise away the console. A flat 256 MB on
    /// a 32 GB machine reserves under one per cent, which is a rounding
    /// error pretending to be a safety margin.
    #[test]
    fn a_node_keeps_enough_of_itself_at_either_end_of_the_range() {
        // The test node: the floor is what protects it.
        let small = 1024 * MB;
        assert_eq!(allocatable_memory(small), small - NODE_RESERVE_FLOOR);
        assert!(
            small - allocatable_memory(small) > small * 15 / 100,
            "the floor wins where the fraction is too little"
        );

        // A big machine: the fraction is what protects it.
        let large = 32 * 1024 * MB;
        assert_eq!(allocatable_memory(large), large - large * 15 / 100);
        assert!(
            large - allocatable_memory(large) > NODE_RESERVE_FLOOR,
            "the fraction wins where the floor is nothing"
        );

        // And a machine smaller than the floor promises nothing rather
        // than wrapping into a very large number.
        assert_eq!(allocatable_memory(128 * MB), 0);
    }

    /// CPU's reserve is flat, because the node's own work does not grow
    /// with the machine: it answers a console and runs a reconcile loop
    /// whether there is one core or thirty-two.
    #[test]
    fn the_cpu_reserve_does_not_grow_with_the_machine() {
        assert_eq!(allocatable_cpu(1_000), 750);
        assert_eq!(allocatable_cpu(32_000), 31_750);
        assert_eq!(allocatable_cpu(100), 0, "and never wraps");
    }

    /// A rung reads the way somebody thinks about it. "Half a core" is
    /// what an operator means; "500 millicores" is what the kernel
    /// means, and the page is not for the kernel.
    #[test]
    fn a_cpu_rung_reads_as_cores() {
        assert_eq!(cpu_label(250), "¼ core");
        assert_eq!(cpu_label(500), "½ core");
        assert_eq!(cpu_label(1_000), "1 core");
        assert_eq!(cpu_label(4_000), "4 cores");
        // Not on the ladder, and still readable — `allocatable_cpu`
        // produces these, and a page showing one must not show a blank.
        assert_eq!(cpu_label(31_750), "31750m");
    }

    /// Empty is "no ceiling", and anything off the ladder is refused —
    /// a field that took any number is a field somebody puts 50 in,
    /// which is a container that cannot finish starting.
    #[test]
    fn a_cpu_ceiling_is_one_of_the_rungs_or_none_at_all() {
        assert_eq!(parse_cpu(""), Ok(None));
        assert_eq!(parse_cpu("  "), Ok(None));
        assert_eq!(parse_cpu("500"), Ok(Some(500)));
        assert!(parse_cpu("50").is_err());
        assert!(parse_cpu("half").is_err());
    }

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
