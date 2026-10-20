use std::fmt;

use serde::Serialize;

use crate::domain::values::hostname::Hostname;

pub const NOT_A_DOMAIN: &str =
    "a domain is a name like example.com — at least two parts separated by a dot";
pub const BAD_LABEL: &str = "a domain is made of letters, digits and hyphens, and no part of it \
     starts or ends with a hyphen or is longer than 63 characters";
pub const WILDCARD_ZONE: &str = "a zone is a domain, not a pattern — add the domain itself";
pub const TOO_LONG: &str = "that name is longer than DNS allows";

/// The apex of a zone, as it is stored and served.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct ZoneName(String);

impl ZoneName {
    pub fn parse(typed: &str) -> Result<Self, &'static str> {
        let name = Hostname::parse(typed);
        if name.is_wildcard() {
            return Err(WILDCARD_ZONE);
        }
        let name = name.as_str();
        if name.len() > 253 {
            return Err(TOO_LONG);
        }
        let labels: Vec<&str> = name.split('.').collect();
        if labels.len() < 2 || labels.iter().any(|label| label.is_empty()) {
            return Err(NOT_A_DOMAIN);
        }
        if !labels.iter().all(|label| is_host_label(label)) {
            return Err(BAD_LABEL);
        }
        Ok(Self(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `name` is this zone or a name inside it.
    pub fn holds(&self, name: &str) -> bool {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        name == self.0 || name.ends_with(&format!(".{}", self.0))
    }
}

impl fmt::Display for ZoneName {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

pub(crate) fn is_host_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_somebody_pastes_becomes_one_zone() {
        for typed in [
            "TeresaAsprilla.com",
            "  teresaasprilla.com. ",
            "https://teresaasprilla.com/",
        ] {
            assert_eq!(
                ZoneName::parse(typed).expect("a zone").as_str(),
                "teresaasprilla.com",
                "{typed}"
            );
        }
    }

    #[test]
    fn a_zone_is_a_registrable_looking_name() {
        assert_eq!(ZoneName::parse("localhost"), Err(NOT_A_DOMAIN));
        assert_eq!(ZoneName::parse(""), Err(NOT_A_DOMAIN));
        assert_eq!(ZoneName::parse("*.example.com"), Err(WILDCARD_ZONE));
        assert_eq!(ZoneName::parse("-bad.example.com"), Err(BAD_LABEL));
        assert_eq!(ZoneName::parse("under_score.com"), Err(BAD_LABEL));
        assert_eq!(
            ZoneName::parse(&format!("{}.com", "a".repeat(64))),
            Err(BAD_LABEL)
        );
        assert!(ZoneName::parse("xn--bcher-kva.example").is_ok());
        assert!(ZoneName::parse("dev.teresaasprilla.com").is_ok());
    }

    /// `notteresaasprilla.com` ends with the same letters and is somebody
    /// else's domain entirely.
    #[test]
    fn a_zone_holds_its_own_names_and_no_others() {
        let zone = ZoneName::parse("teresaasprilla.com").expect("zone");
        assert!(zone.holds("teresaasprilla.com"));
        assert!(zone.holds("WWW.teresaasprilla.com."));
        assert!(zone.holds("a.b.teresaasprilla.com"));
        assert!(!zone.holds("notteresaasprilla.com"));
        assert!(!zone.holds("com"));
    }
}
