//! The console in English or Spanish.
//!
//! ## The English text is the key
//!
//! `t("Add a port")` returns the Spanish for it, or the English back
//! when there is none. Not an enum of symbolic keys, and the reason is
//! the retrofit: there are over a thousand strings in this console, and
//! a key like `NODES_ENROL_HINT` would mean reading a table to find out
//! what a page says. With the English in place, the source still reads
//! as the page reads.
//!
//! The cost is that changing an English word orphans its translation.
//! That is the right way round: an orphan falls back to English, which
//! is a page somebody can still use, and `cargo test` names it.
//!
//! ## Why the language is not passed down
//!
//! It is set around the render, not threaded through it. The
//! alternative is a `Language` parameter on every view, every card and
//! every row helper in this console — a few hundred signatures whose
//! only purpose would be to carry it to the leaves.
//!
//! A render outside the scope reads as English rather than failing.
//! That is the case for tests and for anything rendered off a request,
//! neither of which has a person reading it.
//!
//! ## What is not translated
//!
//! Anything a machine reads or a person types: hostnames, ids, image
//! names, slugs, container states as containerd reports them. And
//! `doctor`, which runs on a terminal and prints what an operator will
//! paste into an issue.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    En,
    Es,
}

impl Language {
    pub fn parse(value: &str) -> Self {
        match value {
            "es" => Self::Es,
            _ => Self::En,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Es => "es",
        }
    }

    /// What the toggle says it is. Two letters, because the control is
    /// the width of the theme's and a language name would not fit —
    /// and because `ES` and `EN` are what somebody is scanning for.
    pub fn short(self) -> &'static str {
        match self {
            Self::En => "EN",
            Self::Es => "ES",
        }
    }

    /// The one the button hands over to. Two states, so it is a toggle
    /// rather than a cycle.
    pub fn other(self) -> Self {
        match self {
            Self::En => Self::Es,
            Self::Es => Self::En,
        }
    }

    /// Said in the language being *offered*, not the current one: the
    /// person reading a tooltip on a button marked `ES` is looking for
    /// confirmation that pressing it gives them Spanish.
    pub fn offer(self) -> &'static str {
        match self {
            Self::En => "Cambiar a español",
            Self::Es => "Switch to English",
        }
    }
}

thread_local! {
    /// The language of the render in progress.
    static CURRENT: std::cell::Cell<Language> = const { std::cell::Cell::new(Language::En) };
}

/// Render with this language in scope.
///
/// Synchronous, and that is the whole design. A task-local would be the
/// obvious shape and cannot work here: the middleware that knows the
/// account returns *before* the handler runs, so it has no future to
/// wrap. A view, on the other hand, ends in one `rsx!{…}.render()` with
/// no `await` inside it — so a thread-local set for exactly that call
/// cannot be observed by another request, because nothing yields while
/// it is set.
///
/// Restores rather than clears, so a nested render — a card inside a
/// frame — leaves the outer one as it found it.
pub fn scoped<T>(language: Language, render: impl FnOnce() -> T) -> T {
    let previous = CURRENT.with(|current| current.replace(language));
    let out = render();
    CURRENT.with(|current| current.set(previous));
    out
}

/// The language of the render in progress, or English.
pub fn current() -> Language {
    CURRENT.with(|current| current.get())
}

/// This text, in the language being rendered.
///
/// English in, English or Spanish out. A string with no entry comes
/// back unchanged — a page in mixed languages is worse than a page in
/// one, and better than a page that panics or shows a key.
pub fn t(english: &'static str) -> &'static str {
    match current() {
        Language::En => english,
        Language::Es => es(english).unwrap_or(english),
    }
}

/// The Spanish for one English string, when there is one.
///
/// A `match` rather than a map: it compiles to a jump table, needs no
/// allocation and no lazy static, and every arm is checked to be a
/// literal at build time. The table lives in `es.rs` because it is
/// long and mechanical, and this module is neither.
fn es(english: &str) -> Option<&'static str> {
    super::es::lookup(english)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_returned_unchanged() {
        assert_eq!(t("Projects"), "Projects");
    }

    /// Outside a request there is nobody reading, so the fallback is
    /// the language the source is written in rather than a panic.
    #[test]
    fn nothing_in_scope_reads_as_english() {
        assert_eq!(current(), Language::En);
    }

    #[test]
    fn spanish_is_looked_up_by_the_english() {
        scoped(Language::Es, || assert_eq!(t("Projects"), "Proyectos"));
    }

    /// A string nobody has translated comes back in English. The page
    /// is then mixed, which is worse than one language and far better
    /// than a key or a panic.
    #[test]
    fn an_untranslated_string_falls_back() {
        scoped(Language::Es, || assert_eq!(t("kubectl"), "kubectl"));
    }

    /// A card rendered inside a frame must leave the frame's language
    /// as it found it, or the rest of the page after it reverts.
    #[test]
    fn a_nested_render_restores_what_it_found() {
        scoped(Language::Es, || {
            scoped(Language::En, || assert_eq!(t("Projects"), "Projects"));
            assert_eq!(t("Projects"), "Proyectos");
        });
    }
}
