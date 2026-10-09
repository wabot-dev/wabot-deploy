pub mod access;
pub mod backups;
pub mod databases;
pub mod network;
pub mod organisations;
pub mod people;
pub mod projects;
pub mod updates;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::tree::{kind_of, rust_files, Kind};

    /// **A file in a feature says what it is in its name**, because a
    /// name is all a guard has to go on once a use case sits beside its
    /// page and its command: an actor prefix, or `_command`, `_request`,
    /// `_response`, `_controller`, `_page` or `_component`. A file with
    /// none of them is one every guard reads as nothing.
    #[test]
    fn every_file_in_a_feature_says_what_it_is() {
        let mut read = 0;
        let mut silent: Vec<String> = Vec::new();
        for path in rust_files(&["src/features"]) {
            if path.file_name().is_some_and(|name| name == "mod.rs") {
                continue;
            }
            read += 1;
            if kind_of(&path).is_none() {
                silent.push(path.display().to_string());
            }
        }
        assert!(
            read > 20,
            "the scan found {read} files, so it is reading the wrong place"
        );
        assert!(
            silent.is_empty(),
            "name these for what they are: {silent:#?}"
        );
    }

    /// **A request or a response is named for its file**, the way a
    /// controller is: `create_database_request.rs` holds
    /// `CreateDatabaseRequest`. Both it and the command it feeds were
    /// `CreateDatabase`, so a use case importing the two had to be read
    /// by its paths to tell which was which.
    #[test]
    fn a_request_or_a_response_is_named_for_its_file() {
        let mut read = 0;
        let mut wrong: Vec<String> = Vec::new();
        for path in rust_files(&["src/features"]) {
            if !matches!(kind_of(&path), Some(Kind::Request | Kind::Response)) {
                continue;
            }
            read += 1;
            let wanted: String = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .split('_')
                .map(|word| {
                    let mut letters = word.chars();
                    letters
                        .next()
                        .map(|first| first.to_uppercase().chain(letters).collect::<String>())
                        .unwrap_or_default()
                })
                .collect();
            let declared = std::fs::read_to_string(&path)
                .expect("read")
                .lines()
                .filter_map(|line| {
                    line.strip_prefix("pub struct ")
                        .or_else(|| line.strip_prefix("pub enum "))
                })
                .any(|rest| {
                    rest.split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                        .is_some_and(|name| name == wanted)
                });
            if !declared {
                wrong.push(format!("{} does not declare {wanted}", path.display()));
            }
        }
        assert!(
            read > 1,
            "the scan found {read} requests and responses, so it is reading the wrong place"
        );
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// A file nobody declared compiles nowhere and drifts with the suite
    /// green, which is how `attention.rs` was once written and left out.
    #[test]
    fn every_file_in_a_feature_is_compiled() {
        let mut undeclared: Vec<String> = Vec::new();
        for path in rust_files(&["src/features"]) {
            let (module, listing) = if path.file_name().is_some_and(|name| name == "mod.rs") {
                let folder = path.parent().expect("a folder");
                if folder == Path::new("src/features") {
                    continue;
                }
                (folder.file_name(), folder.with_file_name("mod.rs"))
            } else {
                (path.file_stem(), path.with_file_name("mod.rs"))
            };
            let module = module.unwrap_or_default().to_string_lossy();
            let declared = std::fs::read_to_string(&listing)
                .expect("every folder has a mod.rs")
                .lines()
                .any(|line| line.trim() == format!("pub mod {module};"));
            if !declared {
                undeclared.push(path.display().to_string());
            }
        }
        assert!(
            undeclared.is_empty(),
            "declare these in their mod.rs: {undeclared:#?}"
        );
    }
}
