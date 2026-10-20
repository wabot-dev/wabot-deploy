use std::fmt;

use serde::Serialize;

use crate::domain::values::zone_name::{ZoneName, TOO_LONG};

pub const BAD_RECORD_NAME: &str = "a record's name is made of letters, digits, hyphens and \
     underscores — @ for the domain itself, and a * only at the very start";

/// Where a record sits, relative to its zone: `@` for the apex.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct RecordName(String);

pub const APEX: &str = "@";

impl RecordName {
    /// A full name typed into the field is taken as meant — people paste
    /// what their mail provider printed, and it prints the whole thing.
    pub fn parse(typed: &str, zone: &ZoneName) -> Result<Self, &'static str> {
        let typed = typed.trim().trim_end_matches('.').to_ascii_lowercase();
        if typed.is_empty() || typed == APEX || typed == zone.as_str() {
            return Ok(Self::apex());
        }
        let relative = typed
            .strip_suffix(&format!(".{}", zone.as_str()))
            .map(str::to_string)
            .unwrap_or(typed);

        if relative.len() + 1 + zone.as_str().len() > 253 {
            return Err(TOO_LONG);
        }
        let labels: Vec<&str> = relative.split('.').collect();
        for (index, label) in labels.iter().enumerate() {
            let wildcard = *label == "*" && index == 0;
            if !wildcard && !is_record_label(label) {
                return Err(BAD_RECORD_NAME);
            }
        }
        Ok(Self(relative))
    }

    pub fn apex() -> Self {
        Self(APEX.to_string())
    }

    pub fn is_apex(&self) -> bool {
        self.0 == APEX
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn fqdn(&self, zone: &ZoneName) -> String {
        match self.is_apex() {
            true => zone.as_str().to_string(),
            false => format!("{}.{}", self.0, zone.as_str()),
        }
    }
}

impl fmt::Display for RecordName {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

fn is_record_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
        && !label.starts_with('-')
        && !label.ends_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone() -> ZoneName {
        ZoneName::parse("teresaasprilla.com").expect("zone")
    }

    #[test]
    fn the_domain_itself_is_the_apex_however_it_is_typed() {
        for typed in ["", "@", " @ ", "teresaasprilla.com", "TeresaAsprilla.com."] {
            assert!(
                RecordName::parse(typed, &zone()).expect(typed).is_apex(),
                "{typed:?}"
            );
        }
    }

    /// Mailgun prints `mg.teresaasprilla.com`; somebody else types `mg`.
    #[test]
    fn a_full_name_inside_the_zone_is_the_same_as_its_relative_part() {
        let pasted =
            RecordName::parse("k1._domainkey.mg.TeresaAsprilla.com.", &zone()).expect("ok");
        let typed = RecordName::parse("k1._domainkey.mg", &zone()).expect("ok");
        assert_eq!(pasted, typed);
        assert_eq!(pasted.fqdn(&zone()), "k1._domainkey.mg.teresaasprilla.com");
    }

    #[test]
    fn what_a_record_name_may_not_be() {
        assert_eq!(RecordName::parse("www.*", &zone()), Err(BAD_RECORD_NAME));
        assert_eq!(RecordName::parse("-www", &zone()), Err(BAD_RECORD_NAME));
        assert_eq!(RecordName::parse("a..b", &zone()), Err(BAD_RECORD_NAME));
        assert_eq!(RecordName::parse("ñandu", &zone()), Err(BAD_RECORD_NAME));
        assert!(RecordName::parse("*", &zone()).is_ok());
        assert!(RecordName::parse("*.blog", &zone()).is_ok());
        assert!(RecordName::parse("_dmarc", &zone()).is_ok());
    }
}
