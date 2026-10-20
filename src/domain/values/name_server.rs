use std::fmt;

use serde::Serialize;

use crate::domain::values::hostname::Hostname;
use crate::domain::values::zone_name::ZoneName;

pub const WILDCARD_NAME_SERVER: &str =
    "a name server is one name, like ns1.example.com — not a pattern";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct NameServerName(String);

impl NameServerName {
    pub fn parse(typed: &str) -> Result<Self, &'static str> {
        if Hostname::parse(typed).is_wildcard() {
            return Err(WILDCARD_NAME_SERVER);
        }
        ZoneName::parse(typed).map(|name| Self(name.as_str().to_string()))
    }

    /// Every name in what was given, in the order given and each once: the
    /// first is the zone's primary in its SOA.
    pub fn all(typed: &[String]) -> Result<Vec<Self>, &'static str> {
        let mut names: Vec<Self> = Vec::new();
        for typed in typed.iter().filter(|typed| !typed.trim().is_empty()) {
            let name = Self::parse(typed)?;
            if !names.contains(&name) {
                names.push(name);
            }
        }
        Ok(names)
    }

    /// What arrived from another node, keeping what parses: a name the
    /// listener cannot write would take the whole zone off the air with it.
    pub fn kept(said: &[String]) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for name in said.iter().filter_map(|said| Self::parse(said).ok()) {
            if !names.contains(&name.0) {
                names.push(name.0);
            }
        }
        names
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NameServerName {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::zone_name::{BAD_LABEL, NOT_A_DOMAIN};

    fn typed(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn names_are_kept_in_order_and_once() {
        let names = NameServerName::all(&typed(&[
            "NS0.nubebasura.lat.",
            "",
            "ns1.nubebasura.lat",
            "ns0.nubebasura.lat",
        ]))
        .expect("two names");
        let names: Vec<&str> = names.iter().map(NameServerName::as_str).collect();
        assert_eq!(names, ["ns0.nubebasura.lat", "ns1.nubebasura.lat"]);
    }

    #[test]
    fn a_name_server_is_one_full_name() {
        assert_eq!(NameServerName::parse("ns1"), Err(NOT_A_DOMAIN));
        assert_eq!(NameServerName::parse("ns_1.example.com"), Err(BAD_LABEL));
        assert_eq!(
            NameServerName::parse("*.example.com"),
            Err(WILDCARD_NAME_SERVER)
        );
        assert_eq!(
            NameServerName::all(&typed(&["ns0.example.com", "ns1"])),
            Err(NOT_A_DOMAIN)
        );
    }

    #[test]
    fn what_another_node_says_keeps_only_what_can_be_served() {
        assert_eq!(
            NameServerName::kept(&typed(&[
                "ns0.example.com",
                "not a name",
                "NS0.example.com",
                "ns1.example.com"
            ])),
            typed(&["ns0.example.com", "ns1.example.com"])
        );
    }
}
