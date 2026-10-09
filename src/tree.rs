//! The source files a guard reads, walked rather than listed, and what
//! each one is, read off its name rather than its directory — because a
//! feature folder holds every kind at once, and a guard that recognised a
//! use case by living in `src/usecases` stopped reading one the moment it
//! moved.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    UseCase,
    Command,
    Request,
    Response,
    Controller,
    Page,
    Component,
    Texts,
}

const ACTORS: &[&str] = &[
    "guest_",
    "node_user_",
    "tenant_user_",
    "cli_user_",
    "remote_node_",
];

const SUFFIXES: &[(&str, Kind)] = &[
    ("_command", Kind::Command),
    ("_request", Kind::Request),
    ("_response", Kind::Response),
    ("_controller", Kind::Controller),
    ("_page", Kind::Page),
    ("_component", Kind::Component),
];

pub fn kind_of(path: &Path) -> Option<Kind> {
    let name = path.file_stem()?.to_str()?;
    if name == "texts" {
        return Some(Kind::Texts);
    }
    if let Some((_, kind)) = SUFFIXES.iter().find(|(suffix, _)| name.ends_with(suffix)) {
        return Some(*kind);
    }
    ACTORS
        .iter()
        .any(|actor| name.starts_with(actor))
        .then_some(Kind::UseCase)
}

pub fn rust_files(directories: &[&str]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending: Vec<PathBuf> = directories.iter().map(PathBuf::from).collect();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("a directory a guard reads") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|kind| kind == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_named_for_its_actor_is_a_page_and_not_a_use_case() {
        assert_eq!(
            kind_of(Path::new("node_user_people_page.rs")),
            Some(Kind::Page)
        );
        assert_eq!(
            kind_of(Path::new("tenant_user_restore_database.rs")),
            Some(Kind::UseCase)
        );
    }

    #[test]
    fn a_file_that_says_nothing_about_itself_has_no_kind() {
        assert_eq!(kind_of(Path::new("fixtures.rs")), None);
        assert_eq!(kind_of(Path::new("node_user.rs")), None);
    }
}
