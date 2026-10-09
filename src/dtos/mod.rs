//! What crosses a use case's boundary.
//!
//! Three kinds, and the split is which direction they travel:
//!
//! - `requests` — what a use case needs to be asked.
//! - `responses` — what it hands back. Never an entity: an entity is a
//!   model with rules and a life, and handing one to a page makes the
//!   page a second place those rules can be read wrong.
//! - the shared ones here, for shapes more than one request or response
//!   carries.
//!
//! **The same DTO crosses both boundaries.** A controller reads a form
//! into a request and hands that request to the use case; the response
//! comes back and the controller renders it. One shape for "what was
//! asked" rather than a form struct, a use case argument list and a page
//! struct that have to be kept agreeing by hand.
//!
//! ## Mapping
//!
//! `wabot::mapper::map` converts between two shapes that hold the same
//! information, **validating on the way**. It agrees on field *names*,
//! not on types, and there is no "just convert it" variant on purpose:
//! a mapping that produced an invalid value would push the error
//! somewhere further away, where it is harder to attribute.
//!
//! It round-trips through `serde_json::Value`, which is right for a
//! request and wrong for a hot loop — write the field-by-field
//! conversion there, where the compiler says which field is missing.

pub mod certificate_facts;
pub mod requests;
pub mod responses;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use crate::tree::{kind_of, rust_files, Kind};

    /// A shape a use case takes or hands back is a DTO, and it lives
    /// here. It was not so once: six `Asked` structs and five outcomes
    /// were declared beside the `run` that used them, which is why the
    /// two consoles' create-service handlers each built their own and a
    /// use case took a `HashMap` and a closure over it — a use case only
    /// a browser could call.
    ///
    /// **It reads the tree rather than a list**, and knows a use case by
    /// its name rather than its directory, so one that moved into a
    /// feature is still read. A guard that names its inputs is right
    /// until somebody adds an input, which has cost this project five
    /// times.
    #[test]
    fn a_use_case_declares_no_shape_of_its_own() {
        let mut declared: BTreeSet<String> = BTreeSet::new();
        let mut read = 0;
        for path in use_cases() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let source = fs::read_to_string(&path).expect("read");
            read += 1;
            for line in source.lines() {
                if line.starts_with("pub struct ") || line.starts_with("pub enum ") {
                    declared.insert(format!("{name}: {}", line.trim()));
                }
            }
        }
        // An empty pass is the failure this cannot otherwise tell from
        // success: a scan pointed somewhere else finds nothing wrong.
        assert!(
            read > 50,
            "the scan found {read} use cases, so it is reading the wrong place"
        );
        assert!(
            declared.is_empty(),
            "these belong in a request or a response of their own: {declared:#?}"
        );
    }

    fn use_cases() -> Vec<std::path::PathBuf> {
        rust_files(&["src"])
            .into_iter()
            .filter(|path| kind_of(path) == Some(Kind::UseCase))
            .collect()
    }

    /// A use case is handed a request, not a form.
    ///
    /// Three of them used to read the browser's own spelling — a
    /// `HashMap` of fields, and once a closure over it — so
    /// `accept-host` and the fact that an unticked checkbox sends
    /// nothing had reached the middle of the application. A use case
    /// that knows those is a use case only a browser can call, which
    /// leaves the CLI and an errand no door at all.
    #[test]
    fn no_use_case_reads_a_form() {
        let mut found: BTreeSet<String> = BTreeSet::new();
        let mut read = 0;
        for path in use_cases() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let source = fs::read_to_string(&path).expect("read");
            read += 1;
            for spelling in ["HashMap", "read_form", "axum", "form_urlencoded"] {
                if source.contains(spelling) {
                    found.insert(format!("{name}: {spelling}"));
                }
            }
        }
        assert!(
            read > 50,
            "the scan found {read} use cases, so it is reading the wrong place"
        );
        assert!(
            found.is_empty(),
            "the controller builds the request; these read the form itself: {found:#?}"
        );
    }

    /// Both directories are wired into the tree. A DTO nobody declared
    /// is a file that compiles nowhere and drifts silently — which is
    /// how `attention.rs` was written, wired and left out with the suite
    /// green.
    #[test]
    fn every_dto_written_is_a_dto_the_tree_knows_about() {
        for (directory, listing) in [
            ("src/dtos/requests", include_str!("requests/mod.rs")),
            ("src/dtos/responses", include_str!("responses/mod.rs")),
        ] {
            for entry in fs::read_dir(directory).expect("the directory is there") {
                let path = entry.expect("entry").path();
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let Some(module) = name.strip_suffix(".rs").filter(|_| name != "mod.rs") else {
                    continue;
                };
                assert!(
                    listing.contains(&format!("pub mod {module};")),
                    "{directory}/{name} is not declared in its mod.rs"
                );
            }
        }
    }
}
