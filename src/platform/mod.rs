//! What the node deploys: projects, and the services under them.
//!
//! ## One containerd namespace, labels for ownership
//!
//! A namespace per project was the obvious shape and is the wrong one.
//! containerd namespaces scope image *content* as well as metadata, so
//! two projects running the same base image would keep two copies of
//! every layer — and the embedded registry that shares the content
//! store, which is the whole storage design, would have nothing to
//! share. What a namespace does not give is network, resource or
//! security isolation.
//!
//! So: one namespace, containers named `<project>--<service>`, and
//! "one project cannot see another's" enforced by this node's API,
//! which is where an authorization rule belongs anyway.
//!
//! Network isolation is a real gap and a separate piece of work — a
//! CNI bridge per project — which lands after services work end to
//! end. Until then containers share the host's network and the node
//! allocates ports.

pub mod access;
pub mod config_history;
pub mod databases;
pub mod edges;
pub mod images;
pub mod ports;
pub mod postgres;
pub mod presets;
pub mod projects;
pub mod registry_credentials;
pub mod releases;
pub mod replicas;
pub mod services;
pub mod tokens;
pub mod volumes;
pub mod wal;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("storage: {0}")]
    Storage(#[from] wabot::sqlite::SqliteError),
    #[error("{0}")]
    Refused(String),
}

pub type PlatformResult<T> = Result<T, PlatformError>;

/// See the note on `AccountError`'s conversion: a refusal is the
/// caller's to fix and says so; a storage failure is ours and does not
/// describe itself over HTTP.
impl From<PlatformError> for wabot::rest::RestError {
    fn from(error: PlatformError) -> Self {
        match error {
            PlatformError::Refused(message) => wabot::rest::RestError::Client {
                status: 400,
                message,
            },
            other => {
                tracing::error!(error = %other, "platform operation failed");
                wabot::rest::RestError::Internal("platform operation failed".into())
            }
        }
    }
}

/// A name a hostname and a container id can be built from.
///
/// Lowercase, digits and hyphens. Runs of anything else collapse to
/// one hyphen and the ends are trimmed, so `"My API (v2)"` is
/// `"my-api-v2"` rather than `"my-api--v2-"`.
pub fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut pending_hyphen = false;

    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_hyphen && !slug.is_empty() {
                slug.push('-');
            }
            pending_hyphen = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            pending_hyphen = true;
        }
    }

    // A DNS label is 63 characters, and this becomes part of one.
    slug.truncate(63);
    slug.trim_end_matches('-').to_string()
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_what_a_hostname_can_hold() {
        assert_eq!(slugify("My API"), "my-api");
        assert_eq!(slugify("My API (v2)"), "my-api-v2");
        assert_eq!(slugify("  spaced  out  "), "spaced-out");
        // Non-ASCII acts as a separator, so a mostly-accented name
        // slugs poorly: `Ünïcødé` becomes `n-c-d`. Transliteration
        // would need a dependency and a table of opinions about which
        // letter `ø` is. The console shows the slug before the service
        // is created, which is the honest fix — the operator sees this
        // and picks a different name.
        assert_eq!(slugify("Ünïcødé"), "n-c-d");
        // The case that actually turns up: an accent inside an
        // otherwise-ASCII word.
        assert_eq!(slugify("Café API"), "caf-api");
        assert_eq!(slugify("already-fine"), "already-fine");
        assert_eq!(slugify("123"), "123");
    }

    /// A run of punctuation must collapse rather than produce a run of
    /// hyphens, and neither end may hold one — `-svc` and `svc-` are
    /// both invalid DNS labels.
    #[test]
    fn separators_collapse_and_the_ends_are_clean() {
        assert_eq!(slugify("a...b"), "a-b");
        assert_eq!(slugify("---a---"), "a");
        assert_eq!(slugify("a - - b"), "a-b");
        assert!(!slugify("trailing!!!").ends_with('-'));
        assert!(!slugify("!!!leading").starts_with('-'));
    }

    #[test]
    fn nothing_sluggable_is_empty_rather_than_a_hyphen() {
        assert_eq!(slugify("???"), "");
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("   "), "");
    }

    /// A DNS label caps at 63, and the truncation must not leave a
    /// hyphen at the end.
    #[test]
    fn a_long_name_is_cut_to_a_valid_label() {
        let slug = slugify(&"a".repeat(100));
        assert_eq!(slug.len(), 63);

        let awkward = slugify(&format!("{}-x", "a".repeat(62)));
        assert!(!awkward.ends_with('-'), "{awkward}");
    }
}
