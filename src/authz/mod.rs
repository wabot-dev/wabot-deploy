//! Who may do what, asked per actor. See `docs/architecture-rules.md` §3.
//!
//! No module here reads a role or a grant table and re-decides it: each
//! composes `crate::policies::access` and `network::capability`. A question per
//! actor; an answer computed once.

pub mod node_user;
pub mod tenant_user;

#[cfg(test)]
mod tests {
    use std::fs;

    fn persistence() -> Vec<(String, String)> {
        let mut found = Vec::new();
        for directory in ["src/accounts", "src/platform"] {
            for entry in fs::read_dir(directory).expect("read the directory") {
                let path = entry.expect("entry").path();
                if path.extension().is_some_and(|kind| kind == "rs") {
                    let name = path
                        .strip_prefix("src")
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    found.push((name, fs::read_to_string(&path).expect("read")));
                }
            }
        }
        assert!(
            found.len() > 15,
            "the scan found {} files, so it is reading the wrong place",
            found.len()
        );
        found
    }

    /// The layer that writes rows answers *what the data may be*, which
    /// is the same answer for every caller. *Who may ask* is the door's
    /// question, and it differs per door — so a persistence module
    /// reaching for `authz` is one of the two sides about to disagree.
    ///
    /// Written as "never imports `authz`" rather than "never calls
    /// `is_admin`", because `invitations::create_into` legitimately reads
    /// the creator's frontier to know which one is theirs. That is data.
    #[test]
    fn nothing_that_writes_a_row_asks_who_is_asking() {
        let offenders: Vec<String> = persistence()
            .into_iter()
            .filter(|(_, source)| {
                let code = source.split("#[cfg(test)]").next().unwrap_or_default();
                code.contains("crate::authz") || code.contains("use super::authz")
            })
            .map(|(name, _)| name)
            .collect();

        assert!(
            offenders.is_empty(),
            "persistence must not authorise, and these reach for authz: {offenders:?}"
        );
    }
}
