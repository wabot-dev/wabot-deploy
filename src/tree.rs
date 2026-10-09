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

const ACTORS: &[&str] = &["guest_", "node_user_", "tenant_user_", "remote_node_"];

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

/// The source with its comments and string literals blanked, so a path
/// written in prose or held in a string is not taken for an import.
pub fn code_only(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        let after_identifier = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if c == 'r' && !after_identifier && matches!(next, Some('"') | Some('#')) {
            let mut hashes = 0;
            let mut j = i + 1;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) != Some(&'"') {
                out.push(c);
                i += 1;
                continue;
            }
            j += 1;
            while j < chars.len()
                && !(chars[j] == '"' && (1..=hashes).all(|k| chars.get(j + k) == Some(&'#')))
            {
                j += 1;
            }
            out.push_str("\"\"");
            i = j + 1 + hashes;
        } else if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            out.push_str("\"\"");
            i += 1;
        } else if c == '\'' && chars.get(i + 2) == Some(&'\'') {
            i += 3;
        } else if c == '\'' && next == Some('\\') {
            i += 2;
            while i < chars.len() && chars[i] != '\'' {
                i += 1;
            }
            i += 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// What ships: every item marked `#[cfg(test)]` taken out. A test may
/// build its world from any room, and that is not a dependency of the
/// thing it tests.
pub fn production(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut lines = code.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "#[cfg(test)]" {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let mut depth = 0;
        for item in lines.by_ref() {
            let trimmed = item.trim();
            if depth == 0 && trimmed.starts_with("#[") {
                continue;
            }
            depth += item.matches('{').count() as i64 - item.matches('}').count() as i64;
            if depth <= 0 && (trimmed.ends_with(';') || trimmed.ends_with('}')) {
                break;
            }
        }
    }
    out
}

/// Every `crate::` path in some code, as its segments, with a `{…}` group
/// opened into one path per name in it.
pub fn crate_paths(code: &str) -> Vec<Vec<String>> {
    fn segments(text: &str) -> (Vec<String>, &str) {
        let mut parts = Vec::new();
        let mut rest = text;
        loop {
            let length = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            if length == 0 {
                break;
            }
            parts.push(rest[..length].to_string());
            rest = &rest[length..];
            match rest.strip_prefix("::") {
                Some(after) if !after.starts_with('{') => rest = after,
                _ => break,
            }
        }
        (parts, rest)
    }

    let mut found = Vec::new();
    let mut rest = code;
    while let Some(at) = rest.find("crate::") {
        let before = rest[..at].chars().last();
        rest = &rest[at + "crate::".len()..];
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let (prefix, after) = segments(rest);
        match after.strip_prefix("::{") {
            Some(group) => {
                let end = group.find('}').unwrap_or(group.len());
                for name in group[..end].split(',') {
                    let (tail, _) = segments(name.trim());
                    if !tail.is_empty() {
                        found.push(prefix.iter().cloned().chain(tail).collect());
                    }
                }
            }
            None => found.push(prefix),
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOMS: &[&str] = &["domain", "infra", "shell", "features", "web", "cli"];

    /// What a room may import: everything beneath it, and nothing above.
    fn may_import(room: &str) -> &'static [&'static str] {
        match room {
            "domain" => &["domain"],
            "infra" => &["domain", "infra"],
            "shell" => &["domain", "infra", "shell"],
            "features" => &["domain", "infra", "shell", "features"],
            "web" => &["domain", "infra", "shell", "features", "web"],
            _ => &["domain", "infra", "shell", "features", "web", "cli"],
        }
    }

    /// **The rooms are a fact the build checks, not a convention.**
    ///
    /// Every production `crate::` path in every file, against the room the
    /// file is in: the domain imports nothing but itself, infrastructure
    /// imports the domain, the shell imports both, a feature imports the
    /// shell and never the web, and the web — the router and the doors no
    /// feature owns — imports everything but the CLI. And
    /// inside a feature, the code a door calls — a use case, a command, a
    /// request or a response — never imports a door, a page, a component
    /// or the shell, with one exception that is part of the rule rather
    /// than beside it: the words, which every sentence somebody reads goes
    /// through.
    ///
    /// Written after the moves that made it true, which found fourteen
    /// imports pointing outward: an entity returning the network's
    /// `Capability`, accounts storing two types that lived in the views,
    /// three calls from beneath a feature into one, and five reads of the
    /// version out of the HTTP surface. None is excused here. And when the
    /// shell was still part of the web, every feature's door imported it;
    /// moving it beneath them is what made "a feature never imports the
    /// web" a rule this can hold.
    #[test]
    fn every_import_points_inward() {
        let mut files = 0;
        let mut checked = 0;
        let mut outward: Vec<String> = Vec::new();
        for path in rust_files(&["src"]) {
            let parts: Vec<String> = path
                .components()
                .skip(1)
                .map(|part| part.as_os_str().to_string_lossy().to_string())
                .collect();
            let room = match parts.first().map(String::as_str) {
                Some(name) if ROOMS.contains(&name) => name.to_string(),
                Some("main.rs") => "cli".to_string(),
                _ => continue,
            };
            files += 1;
            let kind = kind_of(&path);
            let calls_down = room == "features"
                && matches!(
                    kind,
                    Some(Kind::UseCase | Kind::Command | Kind::Request | Kind::Response)
                );
            let source = fs::read_to_string(&path).expect("read");
            for target in crate_paths(&production(&code_only(&source))) {
                let Some(theirs) = target.first() else {
                    continue;
                };
                if !ROOMS.contains(&theirs.as_str()) {
                    continue;
                }
                checked += 1;
                let spelled = format!("crate::{}", target.join("::"));
                if !may_import(&room).contains(&theirs.as_str()) {
                    outward.push(format!("{}: {spelled} ({room} → {theirs})", path.display()));
                    continue;
                }
                if !calls_down {
                    continue;
                }
                let words = target.starts_with(&["shell".into(), "texts".into()]);
                let a_door = theirs == "features"
                    && target.get(2).is_some_and(|module| {
                        matches!(
                            kind_of(Path::new(&format!("{module}.rs"))),
                            Some(Kind::Controller | Kind::Page | Kind::Component)
                        )
                    });
                if (theirs == "shell" && !words) || a_door {
                    let kind = kind.map(|kind| format!("{kind:?}")).unwrap_or_default();
                    outward.push(format!(
                        "{}: {spelled} (a {kind} reaching a door)",
                        path.display()
                    ));
                }
            }
        }
        assert!(
            files > 300 && checked > 1000,
            "read {files} files and {checked} imports, so it is reading the wrong place"
        );
        assert!(
            outward.is_empty(),
            "{} import(s) point outward:\n{}",
            outward.len(),
            outward.join("\n")
        );
    }

    #[test]
    fn a_path_in_a_comment_or_a_string_is_not_an_import() {
        let code = code_only("// crate::web::x\nlet s = \"crate::web::y\"; crate::infra::z();");
        let found = crate_paths(&code);
        assert_eq!(found, vec![vec!["infra".to_string(), "z".to_string()]]);
    }

    #[test]
    fn a_group_is_one_path_per_name() {
        let found = crate_paths("use crate::features::services::{a_component, b_page};");
        assert_eq!(found.len(), 2);
        assert_eq!(found[1], vec!["features", "services", "b_page"]);
    }

    #[test]
    fn what_a_test_imports_does_not_ship() {
        let code = "use crate::infra::x;\n#[cfg(test)]\nmod tests {\n    use crate::web::y;\n}\nfn after() {}\n";
        let kept = production(code);
        assert!(kept.contains("crate::infra::x") && kept.contains("fn after"));
        assert!(!kept.contains("crate::web::y"));
    }

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
