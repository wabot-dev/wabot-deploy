use crate::domain::values::time_zone::TimeZone;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeeServiceStabilityRequest {
    pub project: String,
    pub service: String,
    pub days: u32,
    pub day: Option<i64>,
    pub event: Option<String>,
    pub zone: TimeZone,
}

impl SeeServiceStabilityRequest {
    /// Anything but the two periods the page offers is the shorter one, and
    /// a day that does not read as `YYYY-MM-DD` is no day: a stale link
    /// shows the page rather than a refusal. A zone this binary does not
    /// know is UTC, which the page then says.
    pub fn new(
        project: &str,
        service: &str,
        days: Option<&str>,
        day: Option<&str>,
        event: Option<&str>,
        zone: Option<&str>,
    ) -> Self {
        let zone = zone.and_then(TimeZone::named).unwrap_or_else(TimeZone::utc);
        Self {
            project: project.to_string(),
            service: service.to_string(),
            days: match days {
                Some("90") => 90,
                _ => 30,
            },
            day: day.and_then(|text| parse_day(&zone, text)),
            event: event.filter(|id| !id.is_empty()).map(str::to_string),
            zone,
        }
    }
}

fn parse_day(zone: &TimeZone, text: &str) -> Option<i64> {
    if text.len() != 10 {
        return None;
    }
    zone.midnight(text.parse().ok()?)
}

pub fn day_param(zone: &TimeZone, start: i64) -> String {
    zone.date(start)
        .map(|date| date.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_in_the_address_reads_back_as_the_same_day() {
        let asked = SeeServiceStabilityRequest::new(
            "p",
            "s",
            Some("90"),
            Some("2026-11-21"),
            None,
            Some("America/Bogota"),
        );
        assert_eq!(asked.days, 90);
        assert_eq!(day_param(&asked.zone, asked.day.unwrap()), "2026-11-21");
    }

    #[test]
    fn a_day_in_the_address_is_the_readers_day() {
        let bogota = SeeServiceStabilityRequest::new(
            "p",
            "s",
            None,
            Some("2026-11-21"),
            None,
            Some("America/Bogota"),
        );
        let utc = SeeServiceStabilityRequest::new("p", "s", None, Some("2026-11-21"), None, None);
        assert_eq!(bogota.day.unwrap() - utc.day.unwrap(), 5 * 3_600_000);
    }

    #[test]
    fn a_period_a_day_or_a_zone_the_page_never_offered_falls_back_rather_than_refusing() {
        let asked = SeeServiceStabilityRequest::new(
            "p",
            "s",
            Some("7"),
            Some("yesterday"),
            Some(""),
            Some("Nowhere/At_All"),
        );
        assert_eq!((asked.days, asked.day, asked.event), (30, None, None));
        assert!(asked.zone.reader_unknown());
    }
}
