use serde::Serialize;

use crate::domain::values::record_data::RecordData;
use crate::domain::values::record_name::RecordName;

pub const CNAME_AT_APEX: &str = "the domain itself cannot be a CNAME — it already has the \
     records every domain needs, and a CNAME stands alone. Use an A record";
pub const BAD_TTL: &str = "a TTL is between 60 and 86400 seconds";

pub const MIN_TTL: u32 = 60;
pub const MAX_TTL: u32 = 86_400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DnsRecord {
    pub id: String,
    pub zone_id: String,
    pub name: RecordName,
    pub data: RecordData,
    pub ttl: u32,
}

impl DnsRecord {
    pub fn new(
        id: String,
        zone_id: String,
        name: RecordName,
        data: RecordData,
        ttl: u32,
    ) -> Result<Self, &'static str> {
        if name.is_apex() && matches!(data, RecordData::Cname(_)) {
            return Err(CNAME_AT_APEX);
        }
        if !(MIN_TTL..=MAX_TTL).contains(&ttl) {
            return Err(BAD_TTL);
        }
        Ok(Self {
            id,
            zone_id,
            name,
            data,
            ttl,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::record_data::RecordKind;
    use crate::domain::values::zone_name::ZoneName;

    fn record(
        name: &str,
        kind: RecordKind,
        value: &str,
        ttl: u32,
    ) -> Result<DnsRecord, &'static str> {
        let zone = ZoneName::parse("teresaasprilla.com").expect("zone");
        DnsRecord::new(
            "rec-1".into(),
            "zone-1".into(),
            RecordName::parse(name, &zone).expect("name"),
            RecordData::parse(kind, value, None).expect("data"),
            ttl,
        )
    }

    #[test]
    fn the_apex_cannot_be_a_cname() {
        assert_eq!(
            record("@", RecordKind::Cname, "elsewhere.example.com", 300),
            Err(CNAME_AT_APEX)
        );
        assert!(record("www", RecordKind::Cname, "teresaasprilla.com", 300).is_ok());
    }

    #[test]
    fn a_ttl_is_neither_a_second_nor_a_week() {
        assert_eq!(
            record("@", RecordKind::A, "209.50.48.176", 59),
            Err(BAD_TTL)
        );
        assert_eq!(
            record("@", RecordKind::A, "209.50.48.176", 86_401),
            Err(BAD_TTL)
        );
        assert!(record("@", RecordKind::A, "209.50.48.176", 60).is_ok());
    }
}
