use std::fmt;

/// A hostname as it will be compared and stored.
///
/// Lowercase, no trailing dot, no scheme, no path — all of which
/// somebody pastes, and none of which change which name is meant.
///
/// Normalised **once**, at construction, and immutable after. The edge
/// normalises the `Host` header the same way, and a route stored
/// differently from how it is looked up is a route that never matches.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hostname(String);

/// What somebody who typed a wildcard is told, in every form that takes a
/// name except a service's port, which takes `*.` over a name.
pub const WILDCARD_REFUSED: &str = "a wildcard is a pattern, not a name this node can serve — \
     add each name you want answered";

pub const WILDCARD_SHAPE: &str = "a wildcard is *. in front of a name, like *.apps.example.com — \
     a star anywhere else is not one";

impl Hostname {
    pub fn parse(typed: &str) -> Self {
        let hostname = typed.trim().to_ascii_lowercase();
        let hostname = hostname
            .strip_prefix("https://")
            .or_else(|| hostname.strip_prefix("http://"))
            .unwrap_or(&hostname);
        Self(
            hostname
                .split('/')
                .next()
                .unwrap_or_default()
                .trim_end_matches('.')
                .to_string(),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// A pattern rather than a name. Checked by name wherever one is
    /// typed, never left to DNS: a wildcard record answers for the
    /// literal, so the lookup passed it. Found on a VM node.
    pub fn is_wildcard(&self) -> bool {
        self.0.contains('*')
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn wildcard_base(&self) -> Option<&str> {
        let base = self.0.strip_prefix("*.")?;
        let labelled = !base.is_empty() && !base.contains('*') && base.contains('.');
        labelled.then_some(base)
    }
}

impl fmt::Display for Hostname {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything somebody pastes, and none of it changes which name is
    /// meant.
    #[test]
    fn what_somebody_pastes_becomes_one_name() {
        for typed in [
            "API.Example.COM",
            "  api.example.com  ",
            "https://api.example.com",
            "http://api.example.com/",
            "api.example.com.",
            "api.example.com/some/path",
        ] {
            assert_eq!(
                Hostname::parse(typed).as_str(),
                "api.example.com",
                "{typed}"
            );
        }
    }

    #[test]
    fn a_star_anywhere_is_a_wildcard() {
        assert!(Hostname::parse("*.example.com").is_wildcard());
        assert!(Hostname::parse("api.*.example.com").is_wildcard());
        assert!(!Hostname::parse("api.example.com").is_wildcard());
    }

    #[test]
    fn only_a_leading_star_over_a_real_name_has_a_base() {
        assert_eq!(
            Hostname::parse("*.sub.example.com").wildcard_base(),
            Some("sub.example.com")
        );
        assert_eq!(Hostname::parse("*.com").wildcard_base(), None);
        assert_eq!(Hostname::parse("api.*.example.com").wildcard_base(), None);
        assert_eq!(Hostname::parse("*.*.example.com").wildcard_base(), None);
        assert_eq!(Hostname::parse("*example.com").wildcard_base(), None);
        assert_eq!(Hostname::parse("api.example.com").wildcard_base(), None);
    }

    #[test]
    fn nothing_typed_is_an_empty_name() {
        assert!(Hostname::parse("   ").is_empty());
    }
}
