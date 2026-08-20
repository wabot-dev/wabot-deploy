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

/// The reserve when nobody has chosen one.
///
/// A fraction *and* a floor, for the reason above.
pub fn default_memory_reserve(total: u64) -> u64 {
    (total * NODE_RESERVE_FRACTION / 100).max(NODE_RESERVE_FLOOR)
}

/// How much of a machine may be promised to containers.
///
/// `reserve` is the operator's answer, or `None` for the default. It is a
/// parameter rather than a read inside because this is called from the
/// deploy path and from two pages, and a function that reaches for the
/// database is one that cannot be tested against a number.
pub fn allocatable_memory(total: u64, reserve: Option<u64>) -> u64 {
    total.saturating_sub(reserve.unwrap_or_else(|| default_memory_reserve(total)))
}

/// The same for CPU. `total` is the machine's cores in millicores.
pub fn allocatable_cpu(total: u32, reserve: Option<u32>) -> u32 {
    total.saturating_sub(reserve.unwrap_or(NODE_RESERVE_MILLICORES))
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

/// The smallest ceiling a container can actually run under.
///
/// Docker's own floor, and for the same reason: below a few megabytes the
/// kernel kills the process before it finishes starting. A smaller number
/// is not a tighter limit, it is a service that never comes up — so it is
/// refused with a sentence rather than accepted and wondered about.
pub const MIN_MEMORY: u64 = 6 * MB;

/// And for CPU, a tenth of a core.
///
/// The ladder's own comment recorded that **50 millicores is a container
/// that cannot finish starting** — an argument for a floor rather than for
/// a list, and the only datum this project has about where the floor is.
/// So this is not a measurement: it is the next round number above the one
/// value known to fail. A test asked for 50 to be refused and caught a
/// first version of this that let it through at 10.
///
/// If somebody has a container that starts on less, this is the number to
/// move, and the sentence to replace with what they measured.
pub const MIN_MILLICORES: u32 = 100;

/// A size somebody typed: `512 MB`, `1.5 GB`, `2G`, or `512`.
///
/// Free rather than a rung, because an operator who knows what their
/// service needs should not have to find the nearest power of two. The
/// ladder is still offered — as suggestions beside the field, which is
/// what a list is good for once it stops being the only answer.
///
/// **A bare number is megabytes.** It has to mean *something*, and the
/// alternative reading is bytes: `512` would then be half a kilobyte,
/// which is a ceiling nothing can run under. Between a reading that is
/// sometimes what was meant and one that is never what was meant, this
/// takes the first and the hint beside the field says so.
///
/// Base 1024 throughout, so `1 GB` here is the gibibyte a VPS invoice
/// calls a gigabyte — the same choice the labels already made.
pub fn parse_size(text: &str) -> Result<Option<u64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }

    let digits: String = text
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let unit = text[digits.len()..].trim().to_ascii_lowercase();
    let amount: f64 = digits
        .parse()
        .map_err(|_| format!("{text:?} is not a size — try 512 MB, or 1.5 GB"))?;
    if !amount.is_finite() || amount <= 0.0 {
        return Err(format!("{text:?} is not a size — try 512 MB, or 1.5 GB"));
    }

    // `m` is megabytes here and millicores in the CPU field. Two fields,
    // two parsers, and the same letter: it is what an operator writes in
    // each case, and refusing one spelling to keep the letters distinct
    // would be tidiness paid for by the person typing.
    let multiplier = match unit.as_str() {
        "" | "m" | "mb" | "mib" => MB,
        "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "g" | "gb" | "gib" => 1024 * MB,
        "t" | "tb" | "tib" => 1024 * 1024 * MB,
        _ => return Err(format!("{unit:?} is not a unit — try MB or GB")),
    };

    let bytes = (amount * multiplier as f64).round();
    if bytes > u64::MAX as f64 {
        return Err(format!("{text:?} is larger than any machine"));
    }
    let bytes = bytes as u64;
    if bytes < MIN_MEMORY {
        return Err(format!(
            "{} is under {}, which is less than a container needs to start",
            label(bytes),
            label(MIN_MEMORY)
        ));
    }
    Ok(Some(bytes))
}

/// A CPU ceiling somebody typed: `0.5`, `2`, `1.5`, or `500m`.
///
/// **A bare number is cores and `m` is millicores** — the spelling
/// Kubernetes uses, which is the one an operator who has met this before
/// already knows. It matters that it is not the other way round: the
/// select this replaced posted raw millicores, so reading a bare `1000`
/// as millicores would have kept one string meaning two things depending
/// on which version rendered the page.
pub fn parse_cores(text: &str) -> Result<Option<u32>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }

    let advice = format!("{text:?} is not a CPU size — try 0.5, 2, or 500m");
    let (number, in_millicores) = match text.strip_suffix(['m', 'M']) {
        Some(number) => (number.trim(), true),
        None => (text, false),
    };
    let amount: f64 = number.parse().map_err(|_| advice.clone())?;
    if !amount.is_finite() || amount <= 0.0 {
        return Err(advice);
    }

    let millicores = match in_millicores {
        true => amount.round(),
        false => (amount * 1000.0).round(),
    };
    if millicores > f64::from(u32::MAX) {
        return Err(format!("{text:?} is more CPU than any machine"));
    }
    let millicores = millicores as u32;
    if millicores < MIN_MILLICORES {
        return Err(format!(
            "{} is under {}, which is less than a container needs to start",
            cpu_label(millicores),
            cpu_label(MIN_MILLICORES)
        ));
    }
    Ok(Some(millicores))
}

/// A stored size, spelled so that reading it back gives the same number.
///
/// [`label`] is for prose and rounds — `1536 MB` reads as `1 GB` there,
/// which is fine under a heading and wrong in a field, because saving the
/// page without touching it would quietly change the limit.
pub fn size_field(bytes: u64) -> String {
    let gb = 1024 * MB;
    if bytes.is_multiple_of(gb) {
        return format!("{} GB", bytes / gb);
    }
    if bytes.is_multiple_of(MB) {
        return format!("{} MB", bytes / MB);
    }
    format!("{bytes} B")
}

/// The same for CPU: `1`, `0.5`, `1.25`, or millicores when it is not a
/// clean fraction of a core.
pub fn cores_field(millicores: u32) -> String {
    if millicores.is_multiple_of(1000) {
        return (millicores / 1000).to_string();
    }
    // Two decimals covers every hundredth of a core, which is the floor.
    if millicores.is_multiple_of(10) {
        return format!("{}", f64::from(millicores) / 1000.0);
    }
    format!("{millicores}m")
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
        assert_eq!(allocatable_memory(small, None), small - NODE_RESERVE_FLOOR);
        assert!(
            small - allocatable_memory(small, None) > small * 15 / 100,
            "the floor wins where the fraction is too little"
        );

        // A big machine: the fraction is what protects it.
        let large = 32 * 1024 * MB;
        assert_eq!(allocatable_memory(large, None), large - large * 15 / 100);
        assert!(
            large - allocatable_memory(large, None) > NODE_RESERVE_FLOOR,
            "the fraction wins where the floor is nothing"
        );

        // And a machine smaller than the floor promises nothing rather
        // than wrapping into a very large number.
        assert_eq!(allocatable_memory(128 * MB, None), 0);
    }

    /// CPU's reserve is flat, because the node's own work does not grow
    /// with the machine: it answers a console and runs a reconcile loop
    /// whether there is one core or thirty-two.
    #[test]
    fn the_cpu_reserve_does_not_grow_with_the_machine() {
        assert_eq!(allocatable_cpu(1_000, None), 750);
        assert_eq!(allocatable_cpu(32_000, None), 31_750);
        assert_eq!(allocatable_cpu(100, None), 0, "and never wraps");
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

    /// A bare number is cores and `m` is millicores, and the floor is
    /// what the ladder used to be for.
    ///
    /// The rungs were the rule until Jorge asked for a free choice. The
    /// argument written down for them — a field taking any number is a
    /// field somebody puts 50 in, which is a container that cannot finish
    /// starting — was an argument for a minimum, and 50 is still refused.
    ///
    /// Which way round the bare number reads is the part that matters:
    /// the select this replaced posted raw millicores, so reading `1000`
    /// as millicores would leave one string meaning two things depending
    /// on which version rendered the page. A thousand cores is refused by
    /// the node's own accounting; a thousand millicores silently becomes
    /// a core.
    #[test]
    fn a_cpu_ceiling_is_cores_unless_it_says_millicores() {
        assert_eq!(parse_cores(""), Ok(None));
        assert_eq!(parse_cores("  "), Ok(None));
        assert_eq!(parse_cores("1"), Ok(Some(1_000)));
        assert_eq!(parse_cores("0.5"), Ok(Some(500)));
        assert_eq!(parse_cores("2.25"), Ok(Some(2_250)));
        assert_eq!(parse_cores("500m"), Ok(Some(500)));
        assert_eq!(parse_cores(" 1500m "), Ok(Some(1_500)));
        // The floor: 50 millicores is the value this project recorded as
        // unable to start, and a tenth of a core is what clears it.
        assert_eq!(parse_cores("100m"), Ok(Some(100)));
        assert!(parse_cores("50m").is_err());
        assert!(parse_cores("0").is_err());
        assert!(parse_cores("-2").is_err());
        assert!(parse_cores("half").is_err());
    }

    /// A size somebody typed, in the spellings people type.
    ///
    /// A bare number is megabytes: the other reading is bytes, and `512`
    /// bytes is a ceiling nothing runs under, so it would be a reading
    /// that is never what was meant.
    #[test]
    fn a_size_can_be_typed_in_the_units_people_write() {
        assert_eq!(parse_size(""), Ok(None));
        assert_eq!(parse_size("512"), Ok(Some(512 * MB)));
        assert_eq!(parse_size("512MB"), Ok(Some(512 * MB)));
        assert_eq!(parse_size("512 mb"), Ok(Some(512 * MB)));
        assert_eq!(parse_size("300 MB"), Ok(Some(300 * MB)));
        assert_eq!(parse_size("1G"), Ok(Some(1024 * MB)));
        assert_eq!(parse_size("1.5 GB"), Ok(Some(1536 * MB)));
        assert_eq!(parse_size("1 GiB"), Ok(Some(1024 * MB)));
        // Under what a container needs to start, and not a size at all.
        assert!(parse_size("1MB").is_err());
        assert!(parse_size("0").is_err());
        assert!(parse_size("lots").is_err());
        assert!(parse_size("512 quatloos").is_err());
    }

    /// A field's value read back gives the number that was stored.
    ///
    /// `label` rounds — 1536 MB reads as `1 GB` there, which is right
    /// under a heading and wrong in a field, because saving the page
    /// without touching it would change the limit from 1.5 GB to 1.
    #[test]
    fn a_field_round_trips_what_it_shows() {
        for bytes in [6 * MB, 100 * MB, 300 * MB, 512 * MB, 1536 * MB, 4096 * MB] {
            assert_eq!(
                parse_size(&size_field(bytes)),
                Ok(Some(bytes)),
                "{} did not come back",
                size_field(bytes)
            );
        }
        for millicores in [100, 250, 500, 1_000, 1_250, 4_000, 31_750] {
            assert_eq!(
                parse_cores(&cores_field(millicores)),
                Ok(Some(millicores)),
                "{} did not come back",
                cores_field(millicores)
            );
        }
        // And the rounding one is left alone, because prose wants it.
        assert_eq!(label(1536 * MB), "1 GB");
        assert_eq!(size_field(1536 * MB), "1536 MB");
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
