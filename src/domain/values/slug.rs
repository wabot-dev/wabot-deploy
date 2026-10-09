use std::fmt;

/// A name reduced to what a DNS label can carry.
///
/// Lowercase, digits and hyphens. Runs of anything else collapse to one
/// hyphen and the ends are trimmed, so `"My API (v2)"` is `"my-api-v2"`
/// rather than `"my-api--v2-"`.
///
/// Built once and immutable after: a slug is what a hostname is
/// assembled from, and two callers slugifying the same name slightly
/// differently is a name that resolves nowhere.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slug(String);

impl Slug {
    pub fn of(name: &str) -> Self {
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
        Self(slug.trim_end_matches('-').to_string())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_what_a_hostname_can_hold() {
        assert_eq!(Slug::of("My API").as_str(), "my-api");
        assert_eq!(Slug::of("My API (v2)").as_str(), "my-api-v2");
        assert_eq!(Slug::of("  spaced  out  ").as_str(), "spaced-out");
        // Non-ASCII acts as a separator, so a mostly-accented name
        // slugs poorly: `Ünïcødé` becomes `n-c-d`. Transliteration
        // would need a dependency and a table of opinions about which
        // letter `ø` is. The console shows the slug before the service
        // is created, which is the honest fix — the operator sees this
        // and picks a different name.
        assert_eq!(Slug::of("Ünïcødé").as_str(), "n-c-d");
        // The case that actually turns up: an accent inside an
        // otherwise-ASCII word.
        assert_eq!(Slug::of("Café API").as_str(), "caf-api");
        assert_eq!(Slug::of("already-fine").as_str(), "already-fine");
        assert_eq!(Slug::of("123").as_str(), "123");
    }
    /// A run of punctuation must collapse rather than produce a run of
    /// hyphens, and neither end may hold one — `-svc` and `svc-` are
    /// both invalid DNS labels.
    #[test]
    fn separators_collapse_and_the_ends_are_clean() {
        assert_eq!(Slug::of("a...b").as_str(), "a-b");
        assert_eq!(Slug::of("---a---").as_str(), "a");
        assert_eq!(Slug::of("a - - b").as_str(), "a-b");
        assert!(!Slug::of("trailing!!!").as_str().ends_with('-'));
        assert!(!Slug::of("!!!leading").as_str().starts_with('-'));
    }
    #[test]
    fn nothing_sluggable_is_empty_rather_than_a_hyphen() {
        assert_eq!(Slug::of("???").as_str(), "");
        assert_eq!(Slug::of("").as_str(), "");
        assert_eq!(Slug::of("   ").as_str(), "");
    }
    /// A DNS label caps at 63, and the truncation must not leave a
    /// hyphen at the end.
    #[test]
    fn a_long_name_is_cut_to_a_valid_label() {
        let slug = Slug::of(&"a".repeat(100));
        assert_eq!(slug.as_str().len(), 63);

        let awkward = Slug::of(&format!("{}-x", "a".repeat(62)));
        assert!(!awkward.as_str().ends_with('-'), "{awkward}");
    }
}
