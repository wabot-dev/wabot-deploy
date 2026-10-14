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
/// name.
pub const WILDCARD_REFUSED: &str = "a wildcard is a pattern, not a name this node can serve — \
     add each name you want answered";

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

    /// A pattern rather than a name. Refused by name wherever one is
    /// typed: the edge looks a `Host` up exactly, so a route stored for
    /// `*.example.com` matches nothing, and no authority signs a wildcard
    /// over the HTTP challenge. It was refused only when DNS happened not
    /// to answer for the literal — a wildcard record does. Found on a VM
    /// node.
    pub fn is_wildcard(&self) -> bool {
        self.0.contains('*')
    }

    pub fn as_str(&self) -> &str {
        &self.0
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
    fn nothing_typed_is_an_empty_name() {
        assert!(Hostname::parse("   ").is_empty());
    }
}
