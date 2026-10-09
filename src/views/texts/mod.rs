//! What the console says, split by the view that says it.
//!
//! **The English text is the key.** `t("Add a port")` returns the
//! Spanish, or the English back when there is none — symbolic keys would
//! mean reading a table to find out what a page says.
//!
//! ## One table per view
//!
//! A string lives beside the view that uses it, so changing a page and
//! changing its words is one place, and a view that is deleted takes its
//! words with it. `shared` is for what two views must not spell
//! differently.
//!
//! ## And some texts are functions
//!
//! A sentence carrying a number cannot be a table entry: the number goes
//! in a different place in different languages, and a `format!` at the
//! call site fixes the English order for everybody. Those are functions
//! here — see `sentences`.

pub mod attention;
pub mod language;
pub mod nodes;
pub mod sentences;
pub mod services;
pub mod shared;
pub mod shell;
pub mod shown;

/// Every table, in one place so the lookup cannot forget one.
///
/// Listed rather than discovered, and the test below is what makes that
/// safe: it fails if a module in this directory is missing here. *A
/// guard that names its inputs is right until somebody adds an input* —
/// so this one reads the directory instead.
const TABLES: &[&[(&str, &str)]] = &[
    attention::TABLE,
    crate::features::access::texts::TABLE,
    crate::features::databases::texts::TABLE,
    nodes::TABLE,
    crate::features::organisations::texts::TABLE,
    crate::features::people::texts::TABLE,
    crate::features::projects::texts::TABLE,
    services::TABLE,
    shared::TABLE,
    shell::TABLE,
    shown::TABLE,
    crate::features::updates::texts::TABLE,
];

/// The Spanish for one English string.
///
/// Binary search within each table rather than a `HashMap`: the tables
/// are static and sorted, so this needs no allocation, no lazy
/// initialisation and no hashing — and a page is a few hundred lookups,
/// not a few million.
pub(crate) fn lookup(english: &str) -> Option<&'static str> {
    TABLES.iter().find_map(|table| {
        table
            .binary_search_by(|(key, _)| (*key).cmp(english))
            .ok()
            .map(|at| table[at].1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;

    /// The lookup binary-searches, so an unsorted table silently stops
    /// finding things — the worst possible failure here, because the
    /// page still renders and simply reverts to English.
    #[test]
    fn every_table_is_sorted_and_has_no_duplicates() {
        for table in TABLES {
            for pair in table.windows(2) {
                assert!(
                    pair[0].0 < pair[1].0,
                    "out of order or duplicated: {:?} then {:?}",
                    pair[0].0,
                    pair[1].0
                );
            }
        }
    }

    /// **No English string may appear in two tables.** The lookup takes
    /// the first it finds, so a duplicate is a page whose words change
    /// depending on which table was searched first.
    #[test]
    fn a_string_belongs_to_one_view() {
        let mut seen = BTreeSet::new();
        for table in TABLES {
            for (english, _) in *table {
                assert!(seen.insert(*english), "in two tables: {english:?}");
            }
        }
    }

    /// Reads the directories rather than listing the modules, because a
    /// guard that names its inputs is right until somebody adds an
    /// input — which has cost this project five times. A feature keeps
    /// its words in its own `texts.rs`, so every one of those is a table
    /// too.
    #[test]
    fn the_lookup_searches_every_table_there_is() {
        let mut modules: BTreeSet<String> = fs::read_dir("src/views/texts")
            .expect("read the directory")
            .filter_map(|entry| {
                let path = entry.expect("entry").path();
                let name = path.file_stem()?.to_str()?.to_string();
                (path.extension().is_some_and(|kind| kind == "rs")
                    && !matches!(name.as_str(), "mod" | "language" | "sentences"))
                .then_some(name)
            })
            .collect();
        for path in crate::tree::rust_files(&["src/features"]) {
            if crate::tree::kind_of(&path) != Some(crate::tree::Kind::Texts) {
                continue;
            }
            let module: Vec<String> = path
                .with_extension("")
                .components()
                .skip(1)
                .map(|part| part.as_os_str().to_string_lossy().to_string())
                .collect();
            modules.insert(format!("crate::{}", module.join("::")));
        }

        let listed: BTreeSet<String> = fs::read_to_string("src/views/texts/mod.rs")
            .expect("read this file")
            .lines()
            .filter_map(|line| line.trim().strip_suffix("::TABLE,"))
            .map(str::to_string)
            .collect();

        assert_eq!(
            modules, listed,
            "a text module the lookup never searches renders its whole view in English"
        );
    }

    /// Every file that draws something, read off the disk.
    ///
    /// A directory rather than a list: the old scan named its thirteen
    /// modules and needed a second test to catch a fourteenth, which is
    /// *a guard that names its inputs* wearing a hat. This one cannot
    /// miss a file that exists.
    fn drawn() -> Vec<String> {
        let sources: Vec<String> = crate::tree::rust_files(&[
            "src/console",
            "src/views/components",
            "src/views/layouts",
            "src/views/pages",
            "src/features",
        ])
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("read"))
        .collect();
        assert!(
            sources.len() > 15,
            "the scan found {} files, so it is reading the wrong place",
            sources.len()
        );
        sources
    }

    /// A word that is the same in both languages, listed rather than
    /// allowed silently. Otherwise "translated to itself" cannot tell a
    /// deliberate `Swap` from a line somebody pasted and forgot.
    const THE_SAME_IN_BOTH: &[&str] = &[
        // What a provider's own console calls these fields, in every
        // language: an operator is copying two strings from that screen
        // into this one, and translating the labels would make them stop
        // matching. Same rule as a command not being prose. "Region" is
        // translated because it is a word in a sentence, not a value.
        "Access key id",
        "CPU",
        "Endpoint",
        "Error: ",
        "General",
        // Two protocols, their pair, and an authority's name. A terminal
        // and a certificate say these, and neither speaks Spanish.
        "HTTPS",
        "HTTPS · TCP",
        "Id",
        "Let's Encrypt",
        "Logs",
        "Secret access key",
        // The word for the identifier itself, which is what goes in a
        // URL and in a `docker push` — CLAUDE.md's rule that a slug is
        // not prose, applied to the column that heads a list of them.
        "Slug",
        "Swap",
        "TCP",
    ];

    fn read(path: &str) -> String {
        std::fs::read_to_string(path).expect("read")
    }

    fn calls(source: &str) -> Vec<String> {
        let bytes = source.as_bytes();
        let mut found = Vec::new();
        let mut at = 0;

        while let Some(offset) = source[at..].find("t(\"") {
            let start = at + offset;
            at = start + 3;
            // `post("…")` and `format!("…")` end in `t(` too, so the
            // character before has to be one that cannot be part of a
            // name.
            if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
                continue;
            }
            let mut literal = String::new();
            let mut chars = source[at..].chars().peekable();
            let mut escaped = false;
            let mut closed = false;
            while let Some(c) = chars.next() {
                if escaped {
                    match c {
                        // A continuation: drop it and the indentation.
                        '\n' => {
                            while chars.peek().is_some_and(|c| c.is_whitespace()) {
                                chars.next();
                            }
                        }
                        '"' => literal.push('"'),
                        '\\' => literal.push('\\'),
                        other => literal.push(other),
                    }
                    escaped = false;
                    continue;
                }
                match c {
                    '\\' => escaped = true,
                    '"' => {
                        closed = true;
                        break;
                    }
                    other => literal.push(other),
                }
            }
            if closed {
                found.push(literal);
            }
        }
        found
    }
    #[test]
    fn nothing_is_left_untranslated_by_accident() {
        for (english, spanish) in TABLES.iter().flat_map(|table| table.iter()) {
            assert!(!spanish.is_empty(), "{english:?} has no Spanish");
            if english == spanish {
                assert!(
                    THE_SAME_IN_BOTH.contains(english),
                    "{english:?} is not translated — add the Spanish, or say here \
                     that it is the same word in both"
                );
            }
        }
    }
    /// Every `t("…")` in the console has Spanish behind it.
    ///
    /// The source is read at compile time and scanned, which is the only
    /// way to make "you translated everything" mechanical: a missing
    /// entry does not fail, it quietly renders one line in English on a
    /// page that is otherwise Spanish, and nobody notices until somebody
    /// reading Spanish does.
    ///
    /// A string added to a page and not to the table fails here, by
    /// name. That is the point.
    #[test]
    fn every_string_the_console_asks_for_is_translated() {
        let mut missing = Vec::new();
        for source in drawn() {
            for asked in calls(&source) {
                if lookup(&asked).is_none() {
                    missing.push(asked);
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "no Spanish for {} string(s): {missing:#?}",
            missing.len()
        );
    }
    /// The words a concern is labelled with are translated too.
    ///
    /// **The scan above cannot see them.** It looks for `t("…")`, and a
    /// `Concern`'s `what` is a bare string moved into a field and put
    /// through `language::word` where the card is drawn — deliberately,
    /// because it is chosen in one place and rendered in another. So
    /// every one of those was invisible to the guard that exists to
    /// catch exactly this, and the existing ones are in the table only
    /// because somebody remembered.
    ///
    /// Found by adding one: "Every copy of a database on one machine"
    /// went onto the attention card, the suite stayed green, and a
    /// Spanish console would have shown an English line. A guard that
    /// matches one call shape is right until somebody writes the other
    /// one — the same lesson as the source list below, one level in.
    #[test]
    fn the_words_a_concern_is_labelled_with_are_translated() {
        let source = &read("src/console/attention.rs");
        let mut missing = Vec::new();
        for (index, _) in source.match_indices("what: \"") {
            let rest = &source[index + 7..];
            let Some(end) = rest.find('"') else { continue };
            let word = &rest[..end];
            if lookup(word).is_none() {
                missing.push(word.to_string());
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "no Spanish for {} concern label(s): {missing:#?}",
            missing.len()
        );
    }
    /// Every string that reaches `t` through a *variable* rather than
    /// as a literal.
    ///
    /// `calls` below scans for `t("…")`, which is what almost every
    /// string is — and it cannot see `t(asking(capability))`, where the
    /// English lives in a `match` somewhere else. Adding a capability
    /// put two new strings on the join screen and the suite stayed
    /// green, which is a test passing for a reason that has nothing to
    /// do with the thing being right.
    ///
    /// So the indirect ones are named here by hand. A list that has to
    /// be maintained is worse than one that does not, and it is a great
    /// deal better than a check that quietly stops covering things.
    #[test]
    fn the_words_that_reach_t_through_a_variable_are_translated_too() {
        use crate::network::capability::Capability;

        let mut missing = Vec::new();
        for capability in Capability::ALL {
            for word in [
                crate::views::components::node_capabilities::asking(capability),
                crate::views::components::node_capabilities::why(capability),
                crate::views::components::node_capabilities::offering(capability),
                // And what the same capability is called on this node's
                // own page, which is a different sentence: there it is
                // what another node may ask, here it is what this one
                // does at all. Left out of the first version of this,
                // which is the mistake this test exists to catch.
                crate::views::components::node_capabilities::doing(capability),
                crate::views::components::node_capabilities::doing_why(capability, true),
                crate::views::components::node_capabilities::doing_why(capability, false),
            ] {
                if lookup(word).is_none() {
                    missing.push(word.to_string());
                }
            }
        }
        assert!(missing.is_empty(), "no Spanish for {missing:#?}");
    }
    #[test]
    fn a_string_that_is_there_is_found() {
        assert_eq!(lookup("Projects"), Some("Proyectos"));
        assert_eq!(lookup("nothing like this"), None);
    }
}
