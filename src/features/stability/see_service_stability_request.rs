use crate::domain::policies::stability::DAY_MS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeeServiceStabilityRequest {
    pub project: String,
    pub service: String,
    pub days: u32,
    pub day: Option<i64>,
    pub event: Option<String>,
}

impl SeeServiceStabilityRequest {
    /// Anything but the two periods the page offers is the shorter one, and
    /// a day that does not read as `YYYY-MM-DD` is no day: a stale link
    /// shows the page rather than a refusal.
    pub fn new(
        project: &str,
        service: &str,
        days: Option<&str>,
        day: Option<&str>,
        event: Option<&str>,
    ) -> Self {
        Self {
            project: project.to_string(),
            service: service.to_string(),
            days: match days {
                Some("90") => 90,
                _ => 30,
            },
            day: day.and_then(parse_day),
            event: event.filter(|id| !id.is_empty()).map(str::to_string),
        }
    }
}

fn parse_day(text: &str) -> Option<i64> {
    let format = time::macros::format_description!("[year]-[month]-[day]");
    let date = time::Date::parse(text, &format).ok()?;
    Some(date.midnight().assume_utc().unix_timestamp() * 1000)
}

pub fn day_param(start: i64) -> String {
    let day = start - start.rem_euclid(DAY_MS);
    time::OffsetDateTime::from_unix_timestamp(day / 1000)
        .map(|moment| {
            format!(
                "{:04}-{:02}-{:02}",
                moment.year(),
                u8::from(moment.month()),
                moment.day()
            )
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_in_the_address_reads_back_as_the_same_day() {
        let asked = SeeServiceStabilityRequest::new("p", "s", Some("90"), Some("2026-11-21"), None);
        assert_eq!(asked.days, 90);
        assert_eq!(day_param(asked.day.unwrap()), "2026-11-21");
    }

    #[test]
    fn a_period_or_a_day_the_page_never_offered_falls_back_rather_than_refusing() {
        let asked =
            SeeServiceStabilityRequest::new("p", "s", Some("7"), Some("yesterday"), Some(""));
        assert_eq!((asked.days, asked.day, asked.event), (30, None, None));
    }
}
