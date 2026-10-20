use std::net::{Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Serialize};

use crate::domain::values::zone_name::is_host_label;

pub const NOT_AN_IPV4: &str = "an A record points at an IPv4 address, like 203.0.113.7";
pub const NOT_AN_IPV6: &str = "an AAAA record points at an IPv6 address, like 2001:db8::7";
pub const NOT_A_TARGET: &str =
    "that has to be a full name, like mail.example.com — a name without a dot is ambiguous";
pub const NO_PRIORITY: &str = "a priority is a number from 0 to 65535 — lower is tried first";
pub const EMPTY_TEXT: &str = "a TXT record needs some text";
pub const TEXT_TOO_LONG: &str = "a TXT record this long would not fit in a DNS answer";
pub const BAD_SRV: &str =
    "an SRV record is a weight, a port and a target, like 5 5060 sip.example.com";
pub const BAD_CAA: &str =
    "a CAA record is issue or issuewild and an authority, like issue letsencrypt.org";
pub const UNKNOWN_KIND: &str = "that is not a kind of record this node serves";

const TEXT_LIMIT: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RecordKind {
    A,
    Aaaa,
    Cname,
    Mx,
    Txt,
    Srv,
    Caa,
}

impl RecordKind {
    pub const ALL: [RecordKind; 7] = [
        RecordKind::A,
        RecordKind::Aaaa,
        RecordKind::Cname,
        RecordKind::Mx,
        RecordKind::Txt,
        RecordKind::Srv,
        RecordKind::Caa,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            RecordKind::A => "A",
            RecordKind::Aaaa => "AAAA",
            RecordKind::Cname => "CNAME",
            RecordKind::Mx => "MX",
            RecordKind::Txt => "TXT",
            RecordKind::Srv => "SRV",
            RecordKind::Caa => "CAA",
        }
    }

    pub fn parse(name: &str) -> Result<Self, &'static str> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name().eq_ignore_ascii_case(name.trim()))
            .ok_or(UNKNOWN_KIND)
    }
}

/// What a record says, parsed once. `value` and `priority` are how it is
/// stored, and parsing them back gives the same record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RecordData {
    A(Ipv4Addr),
    Aaaa(Ipv6Addr),
    Cname(String),
    Mx {
        priority: u16,
        exchange: String,
    },
    Txt(String),
    Srv {
        priority: u16,
        weight: u16,
        port: u16,
        target: String,
    },
    Caa {
        critical: bool,
        tag: String,
        value: String,
    },
}

impl RecordData {
    pub fn parse(
        kind: RecordKind,
        value: &str,
        priority: Option<&str>,
    ) -> Result<Self, &'static str> {
        let value = value.trim();
        match kind {
            RecordKind::A => value.parse().map(RecordData::A).map_err(|_| NOT_AN_IPV4),
            RecordKind::Aaaa => value.parse().map(RecordData::Aaaa).map_err(|_| NOT_AN_IPV6),
            RecordKind::Cname => target(value).map(RecordData::Cname),
            RecordKind::Mx => Ok(RecordData::Mx {
                priority: priority_of(priority)?,
                exchange: target(value)?,
            }),
            RecordKind::Txt => text(value).map(RecordData::Txt),
            RecordKind::Srv => {
                let parts: Vec<&str> = value.split_whitespace().collect();
                let [weight, port, host] = parts.as_slice() else {
                    return Err(BAD_SRV);
                };
                Ok(RecordData::Srv {
                    priority: priority_of(priority)?,
                    weight: weight.parse().map_err(|_| BAD_SRV)?,
                    port: port.parse().map_err(|_| BAD_SRV)?,
                    target: target(host)?,
                })
            }
            RecordKind::Caa => caa(value),
        }
    }

    pub fn kind(&self) -> RecordKind {
        match self {
            RecordData::A(_) => RecordKind::A,
            RecordData::Aaaa(_) => RecordKind::Aaaa,
            RecordData::Cname(_) => RecordKind::Cname,
            RecordData::Mx { .. } => RecordKind::Mx,
            RecordData::Txt(_) => RecordKind::Txt,
            RecordData::Srv { .. } => RecordKind::Srv,
            RecordData::Caa { .. } => RecordKind::Caa,
        }
    }

    pub fn value(&self) -> String {
        match self {
            RecordData::A(address) => address.to_string(),
            RecordData::Aaaa(address) => address.to_string(),
            RecordData::Cname(target) => target.clone(),
            RecordData::Mx { exchange, .. } => exchange.clone(),
            RecordData::Txt(text) => text.clone(),
            RecordData::Srv {
                weight,
                port,
                target,
                ..
            } => format!("{weight} {port} {target}"),
            RecordData::Caa {
                critical,
                tag,
                value,
            } => format!("{} {tag} {value}", if *critical { 128 } else { 0 }),
        }
    }

    pub fn priority(&self) -> Option<u16> {
        match self {
            RecordData::Mx { priority, .. } | RecordData::Srv { priority, .. } => Some(*priority),
            _ => None,
        }
    }

    /// The strings a TXT record goes out as. DNS caps one string at 255
    /// bytes, and a DKIM key is longer than that, so it is cut here rather
    /// than refused — which is what every provider does with what it is
    /// pasted.
    pub fn text_strings(text: &str) -> Vec<String> {
        let mut strings = Vec::new();
        let mut current = String::new();
        for character in text.chars() {
            if current.len() + character.len_utf8() > 255 {
                strings.push(std::mem::take(&mut current));
            }
            current.push(character);
        }
        if !current.is_empty() || strings.is_empty() {
            strings.push(current);
        }
        strings
    }
}

fn priority_of(priority: Option<&str>) -> Result<u16, &'static str> {
    priority
        .map(str::trim)
        .filter(|priority| !priority.is_empty())
        .ok_or(NO_PRIORITY)?
        .parse()
        .map_err(|_| NO_PRIORITY)
}

fn target(typed: &str) -> Result<String, &'static str> {
    let name = typed.trim().trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = name.split('.').collect();
    let valid = labels.len() >= 2
        && name.len() <= 253
        && labels
            .iter()
            .all(|label| is_host_label(&label.replace('_', "a")));
    match valid {
        true => Ok(name),
        false => Err(NOT_A_TARGET),
    }
}

fn text(typed: &str) -> Result<String, &'static str> {
    let unquoted = match typed.len() >= 2 && typed.starts_with('"') && typed.ends_with('"') {
        true => typed[1..typed.len() - 1].replace("\" \"", ""),
        false => typed.to_string(),
    };
    if unquoted.is_empty() {
        return Err(EMPTY_TEXT);
    }
    if unquoted.len() > TEXT_LIMIT {
        return Err(TEXT_TOO_LONG);
    }
    Ok(unquoted)
}

fn caa(typed: &str) -> Result<RecordData, &'static str> {
    let mut parts: Vec<&str> = typed.split_whitespace().collect();
    let critical = match parts.first().map(|first| first.parse::<u8>()) {
        Some(Ok(flags)) => {
            parts.remove(0);
            flags & 128 != 0
        }
        _ => false,
    };
    let [tag, value @ ..] = parts.as_slice() else {
        return Err(BAD_CAA);
    };
    let tag = tag.to_ascii_lowercase();
    let value = value.join(" ").trim_matches('"').to_string();
    if !matches!(tag.as_str(), "issue" | "issuewild") || value.is_empty() {
        return Err(BAD_CAA);
    }
    Ok(RecordData::Caa {
        critical,
        tag,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(kind: RecordKind, value: &str, priority: Option<&str>) -> RecordData {
        RecordData::parse(kind, value, priority).expect(value)
    }

    /// What is stored has to come back as the same record, or a zone sent
    /// to another node is a different zone when it arrives.
    #[test]
    fn a_stored_record_reads_back_as_itself() {
        for (kind, value, priority) in [
            (RecordKind::A, "209.50.48.176", None),
            (RecordKind::Aaaa, "2001:db8::7", None),
            (RecordKind::Cname, "mailgun.org", None),
            (RecordKind::Mx, "mxa.mailgun.org", Some("10")),
            (RecordKind::Txt, "v=spf1 include:mailgun.org ~all", None),
            (RecordKind::Srv, "5 5060 sip.example.com", Some("10")),
            (RecordKind::Caa, "0 issue letsencrypt.org", None),
        ] {
            let record = parsed(kind, value, priority);
            let stored = (record.value(), record.priority().map(|p| p.to_string()));
            let again = parsed(kind, &stored.0, stored.1.as_deref());
            assert_eq!(record, again, "{value}");
        }
    }

    /// Mailgun and Google show TXT values in quotes, and a DKIM key as
    /// several quoted strings. All of it is one text to the person pasting.
    #[test]
    fn a_pasted_txt_loses_its_quotes() {
        assert_eq!(
            parsed(RecordKind::Txt, "\"v=spf1 include:mailgun.org ~all\"", None),
            RecordData::Txt("v=spf1 include:mailgun.org ~all".into())
        );
        assert_eq!(
            parsed(RecordKind::Txt, "\"k=rsa; p=AAA\" \"BBB\"", None),
            RecordData::Txt("k=rsa; p=AAABBB".into())
        );
        assert_eq!(
            RecordData::parse(RecordKind::Txt, "  ", None),
            Err(EMPTY_TEXT)
        );
    }

    #[test]
    fn a_txt_longer_than_one_string_is_cut_into_several() {
        let key = "p=".to_string() + &"A".repeat(600);
        let strings = RecordData::text_strings(&key);
        assert_eq!(strings.len(), 3);
        assert!(strings.iter().all(|string| string.len() <= 255));
        assert_eq!(strings.concat(), key);
        assert_eq!(RecordData::text_strings("short"), vec!["short".to_string()]);
    }

    #[test]
    fn what_each_kind_refuses() {
        assert_eq!(
            RecordData::parse(RecordKind::A, "300.1.1.1", None),
            Err(NOT_AN_IPV4)
        );
        assert_eq!(
            RecordData::parse(RecordKind::A, "::1", None),
            Err(NOT_AN_IPV4)
        );
        assert_eq!(
            RecordData::parse(RecordKind::Aaaa, "1.2.3.4", None),
            Err(NOT_AN_IPV6)
        );
        assert_eq!(
            RecordData::parse(RecordKind::Cname, "mail", None),
            Err(NOT_A_TARGET)
        );
        assert_eq!(
            RecordData::parse(RecordKind::Mx, "mxa.mailgun.org", None),
            Err(NO_PRIORITY)
        );
        assert_eq!(
            RecordData::parse(RecordKind::Mx, "mxa.mailgun.org", Some("high")),
            Err(NO_PRIORITY)
        );
        assert_eq!(
            RecordData::parse(RecordKind::Srv, "5060 sip.example.com", Some("1")),
            Err(BAD_SRV)
        );
        assert_eq!(
            RecordData::parse(RecordKind::Caa, "letsencrypt.org", None),
            Err(BAD_CAA)
        );
        assert_eq!(
            RecordData::parse(RecordKind::Txt, &"x".repeat(TEXT_LIMIT + 1), None),
            Err(TEXT_TOO_LONG)
        );
        assert_eq!(RecordKind::parse("PTR"), Err(UNKNOWN_KIND));
        assert_eq!(RecordKind::parse("mx"), Ok(RecordKind::Mx));
    }

    #[test]
    fn a_target_keeps_underscores_and_loses_its_dot() {
        assert_eq!(
            parsed(RecordKind::Cname, "_domainkey.Example.com.", None),
            RecordData::Cname("_domainkey.example.com".into())
        );
    }
}
