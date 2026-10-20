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

pub mod language;
pub mod nodes;
pub mod sentences;
pub mod shared;
pub mod shell;
pub mod shown;

/// What every feature's words fall back to: the shell's, and what more
/// than one feature says. A feature's own table is searched first, by
/// its own `texts::t`, so nothing here has to list the features.
const SHARED: &[&[(&str, &str)]] = &[nodes::TABLE, shared::TABLE, shell::TABLE, shown::TABLE];

/// The Spanish for one English string: in the caller's own table, then
/// in the shared ones.
///
/// Binary search within each table rather than a `HashMap`: the tables
/// are static and sorted, so this needs no allocation, no lazy
/// initialisation and no hashing — and a page is a few hundred lookups,
/// not a few million.
pub(crate) fn lookup(english: &str, own: &[(&'static str, &'static str)]) -> Option<&'static str> {
    std::iter::once(own)
        .chain(SHARED.iter().copied())
        .find_map(|table| {
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

    /// Every table there is, by name: the shared ones here and each
    /// feature's own. Listed because the guards need the values;
    /// `every_table_there_is_one_the_guards_read` compares it with the disk.
    fn every_table() -> Vec<(&'static str, &'static [(&'static str, &'static str)])> {
        vec![
            ("nodes", nodes::TABLE),
            ("shared", shared::TABLE),
            ("shell", shell::TABLE),
            ("shown", shown::TABLE),
            ("access", crate::features::access::texts::TABLE),
            ("backups", crate::features::backups::texts::TABLE),
            ("databases", crate::features::databases::texts::TABLE),
            ("dns", crate::features::dns::texts::TABLE),
            ("network", crate::features::network::texts::TABLE),
            ("node", crate::features::node::texts::TABLE),
            (
                "organisations",
                crate::features::organisations::texts::TABLE,
            ),
            ("people", crate::features::people::texts::TABLE),
            ("projects", crate::features::projects::texts::TABLE),
            ("services", crate::features::services::texts::TABLE),
            ("updates", crate::features::updates::texts::TABLE),
        ]
    }

    fn own_table(feature: &str) -> &'static [(&'static str, &'static str)] {
        every_table()
            .into_iter()
            .find(|(name, _)| *name == feature)
            .map(|(_, table)| table)
            .expect("a feature with words has a table")
    }

    /// Whose words a file says: its feature's, or the shared ones.
    fn own_table_of(path: &std::path::Path) -> &'static [(&'static str, &'static str)] {
        match path
            .strip_prefix("src/features")
            .ok()
            .and_then(|rest| rest.components().next())
        {
            Some(feature) if rest_is_a_folder(path) => {
                own_table(&feature.as_os_str().to_string_lossy())
            }
            _ => &[],
        }
    }

    fn rest_is_a_folder(path: &std::path::Path) -> bool {
        path.parent() != Some(std::path::Path::new("src/features"))
    }

    /// The lookup binary-searches, so an unsorted table silently stops
    /// finding things — the worst possible failure here, because the
    /// page still renders and simply reverts to English.
    #[test]
    fn every_table_is_sorted_and_has_no_duplicates() {
        for (_, table) in every_table() {
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
        for (_, table) in every_table() {
            for (english, _) in table {
                assert!(seen.insert(*english), "in two tables: {english:?}");
            }
        }
    }

    /// Every table on the disk is one the guards read, and every shared
    /// one is one the lookup falls back to. A table nobody checks is one
    /// that can fall out of order and stop finding things, and the page
    /// still renders — in English.
    #[test]
    fn every_table_there_is_one_the_guards_read() {
        let shared: BTreeSet<String> = fs::read_dir("src/shell/texts")
            .expect("read the directory")
            .filter_map(|entry| {
                let path = entry.expect("entry").path();
                let name = path.file_stem()?.to_str()?.to_string();
                (path.extension().is_some_and(|kind| kind == "rs")
                    && !matches!(name.as_str(), "mod" | "language" | "sentences"))
                .then_some(name)
            })
            .collect();
        let mut on_disk = shared.clone();
        for path in crate::tree::rust_files(&["src/features"]) {
            if crate::tree::kind_of(&path) == Some(crate::tree::Kind::Texts) {
                let feature = path.parent().and_then(|folder| folder.file_name());
                on_disk.insert(feature.unwrap_or_default().to_string_lossy().to_string());
            }
        }
        let listed: BTreeSet<String> = every_table()
            .into_iter()
            .map(|(name, _)| name.to_string())
            .collect();
        assert_eq!(
            on_disk, listed,
            "a table the guards do not read can drift unseen"
        );
        assert_eq!(
            SHARED.len(),
            shared.len(),
            "a shared table the lookup does not fall back to renders in English"
        );
    }

    /// **A feature asks for its words with its own `t`.** The shell's `t`
    /// looks only in the shared tables, so a feature's page that called it
    /// would draw every word of its own in English, while every check of
    /// the tables still passed — which is how the projects list went out
    /// when the words moved into the features: the one file the rewrite
    /// missed, found later by hand. This reads every feature file but the
    /// feature's own `texts.rs` for the shell's two doors into the tables.
    #[test]
    fn a_feature_asks_for_its_words_by_its_own_t() {
        let mut read = 0;
        let mut wrong = Vec::new();
        for path in crate::tree::rust_files(&["src/features"]) {
            let in_a_feature = path.parent() != Some(std::path::Path::new("src/features"));
            if !in_a_feature || crate::tree::kind_of(&path) == Some(crate::tree::Kind::Texts) {
                continue;
            }
            read += 1;
            let source = fs::read_to_string(&path).expect("read");
            let code = crate::tree::production(&crate::tree::code_only(&source));
            for target in crate::tree::crate_paths(&code) {
                let into_the_tables =
                    target.starts_with(&["shell".into(), "texts".into(), "language".into()])
                        && matches!(target.get(3).map(String::as_str), Some("t" | "word"));
                if into_the_tables {
                    wrong.push(format!("{}: crate::{}", path.display(), target.join("::")));
                }
            }
            for call in ["language::t(", "language::word("] {
                if code.contains(call) {
                    wrong.push(format!("{}: {call}", path.display()));
                }
            }
        }
        assert!(
            read > 150,
            "the scan read {read} feature files, so it is reading the wrong place"
        );
        assert!(
            wrong.is_empty(),
            "these ask the shell for a feature's words, which it does not hold: {wrong:#?}"
        );
    }

    /// Every file that draws something, read off the disk.
    ///
    /// A directory rather than a list: the old scan named its thirteen
    /// modules and needed a second test to catch a fourteenth, which is
    /// *a guard that names its inputs* wearing a hat. This one cannot
    /// miss a file that exists.
    fn drawn() -> Vec<(std::path::PathBuf, String)> {
        let sources: Vec<(std::path::PathBuf, String)> =
            crate::tree::rust_files(&["src/shell", "src/web", "src/features"])
                .into_iter()
                // The tables and the lookup are what the scan checks against, and
                // their docs quote `t("…")` as an example, untranslated on purpose.
                .filter(|path| !path.starts_with("src/shell/texts"))
                .map(|path| {
                    let source = std::fs::read_to_string(&path).expect("read");
                    (path, source)
                })
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
        // The protocol, and the field every DNS provider calls this: a
        // record is copied from a provider's screen, and the labels have to
        // match it.
        "DNS",
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
        "TTL",
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
        for (english, spanish) in every_table()
            .into_iter()
            .flat_map(|(_, table)| table.iter())
        {
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
        for (path, source) in drawn() {
            let own = own_table_of(&path);
            for asked in calls(&source) {
                if lookup(&asked, own).is_none() {
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
    /// `ConcernResponse`'s `what` is a bare string moved into a field and put
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
        let source = &read("src/features/node/node_user_see_what_needs_attention.rs");
        let mut missing = Vec::new();
        for (index, _) in source.match_indices("what: \"") {
            let rest = &source[index + 7..];
            let Some(end) = rest.find('"') else { continue };
            let word = &rest[..end];
            if lookup(word, own_table("node")).is_none() {
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
        use crate::domain::values::capability::Capability;

        let mut missing = Vec::new();
        for capability in Capability::ALL {
            for word in [
                crate::shell::components::node_capabilities::asking(capability),
                crate::shell::components::node_capabilities::why(capability),
                crate::shell::components::node_capabilities::offering(capability),
                // And what the same capability is called on this node's
                // own page, which is a different sentence: there it is
                // what another node may ask, here it is what this one
                // does at all. Left out of the first version of this,
                // which is the mistake this test exists to catch.
                crate::shell::components::node_capabilities::doing(capability),
                crate::shell::components::node_capabilities::doing_why(capability, true),
                crate::shell::components::node_capabilities::doing_why(capability, false),
            ] {
                if lookup(word, &[]).is_none() {
                    missing.push(word.to_string());
                }
            }
        }
        assert!(missing.is_empty(), "no Spanish for {missing:#?}");
    }
    #[test]
    fn a_string_that_is_there_is_found() {
        assert_eq!(lookup("Projects", &[]), Some("Proyectos"));
        assert_eq!(lookup("nothing like this", &[]), None);
    }
}
