//! The texts that are computed, because they carry a number or a name.
//!
//! **A sentence with a figure in it cannot be a table entry.** The
//! figure goes in a different place in different languages, and a
//! `format!` at the call site fixes the English word order for
//! everybody. Building the sentence here is what lets the Spanish put
//! the number where Spanish wants it.
//!
//! Each of these is one function, and the two languages are two arms of
//! one `match` — so a translation that forgets the number is a compile
//! error rather than a page with a hole in it.

use super::language::{current, Language};

/// What a drain did, and what it could not do.
pub fn copies_moved(moved: usize, refused: &[String]) -> String {
    match current() {
        Language::En => match refused.is_empty() {
            true => format!("{moved} copies moved."),
            false => format!("{moved} copies moved; {}", refused.join("; ")),
        },
        Language::Es => match refused.is_empty() {
            true => format!("Se movieron {moved} copias."),
            false => format!("Se movieron {moved} copias; {}", refused.join("; ")),
        },
    }
}

/// Why a node cannot stop owning things yet.
pub fn still_owns_projects(count: usize) -> String {
    match current() {
        Language::En => {
            format!("this node owns {count} project(s) of its own — move or delete them first")
        }
        Language::Es => {
            format!("este nodo tiene {count} proyecto(s) propios — muévelos o elimínalos primero")
        }
    }
}

/// A name another organisation already answers on.
pub fn name_is_taken(holder: &str) -> String {
    match current() {
        Language::En => format!("{holder} already answers on that name"),
        Language::Es => format!("{holder} ya responde en ese nombre"),
    }
}

/// A release this node is already running.
pub fn already_running(tag: &str) -> String {
    match current() {
        Language::En => format!("{tag} is what this node is already running"),
        Language::Es => format!("{tag} es lo que este nodo ya está ejecutando"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The number moves. That is the whole reason these are functions:
    /// Spanish puts the count after the verb, and a `format!` at the
    /// call site would have fixed the English order for both.
    #[test]
    fn the_figure_sits_where_each_language_wants_it() {
        let english = super::super::language::scoped(Language::En, || copies_moved(2, &[]));
        let spanish = super::super::language::scoped(Language::Es, || copies_moved(2, &[]));

        assert!(english.starts_with('2'), "{english}");
        assert!(spanish.starts_with("Se movieron"), "{spanish}");
    }
}
