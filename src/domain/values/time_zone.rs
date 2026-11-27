use jiff::civil::Date;
use jiff::{tz, Timestamp, ZonedDateTime};

const DAY_MS: i64 = 86_400_000;

/// Where a reader's days begin and end.
///
/// The browser names it; a page that never heard from one counts in UTC
/// and has to say so, because a time with no zone beside it reads as the
/// reader's own.
#[derive(Debug, Clone)]
pub struct TimeZone {
    name: Option<String>,
    rules: tz::TimeZone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    pub year: i16,
    pub month: u8,
    pub day: u8,
    pub weekday: u8,
    pub hour: u8,
    pub minute: u8,
}

impl TimeZone {
    pub fn utc() -> Self {
        Self {
            name: None,
            rules: tz::TimeZone::UTC,
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        tz::TimeZone::get(name).ok().map(|rules| Self {
            name: Some(name.to_string()),
            rules,
        })
    }

    pub fn reader_unknown(&self) -> bool {
        self.name.is_none()
    }

    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or("UTC")
    }

    fn zoned(&self, at: i64) -> Option<ZonedDateTime> {
        Timestamp::from_millisecond(at)
            .ok()
            .map(|moment| moment.to_zoned_datetime(self.rules.clone()))
    }

    pub fn day_of(&self, at: i64) -> i64 {
        self.zoned(at)
            .and_then(|zoned| zoned.start_of_day().ok())
            .map(|start| start.timestamp().as_millisecond())
            .unwrap_or(at - at.rem_euclid(DAY_MS))
    }

    pub fn next_day(&self, at: i64) -> i64 {
        self.date(at)
            .and_then(|date| date.tomorrow().ok())
            .and_then(|date| self.midnight(date))
            .unwrap_or(self.day_of(at) + DAY_MS)
    }

    pub fn previous_day(&self, at: i64) -> i64 {
        self.date(at)
            .and_then(|date| date.yesterday().ok())
            .and_then(|date| self.midnight(date))
            .unwrap_or(self.day_of(at) - DAY_MS)
    }

    pub fn midnight(&self, date: Date) -> Option<i64> {
        date.to_zoned_datetime(self.rules.clone())
            .ok()
            .map(|start| start.timestamp().as_millisecond())
    }

    pub fn hour_on(&self, day: i64, hour: i8) -> Option<i64> {
        self.date(day)?
            .at(hour, 0, 0, 0)
            .to_zoned_datetime(self.rules.clone())
            .ok()
            .map(|moment| moment.timestamp().as_millisecond())
    }

    pub fn date(&self, at: i64) -> Option<Date> {
        self.zoned(at).map(|zoned| zoned.date())
    }

    pub fn moment(&self, at: i64) -> Option<Moment> {
        let zoned = self.zoned(at)?;
        Some(Moment {
            year: zoned.year(),
            month: zoned.month() as u8,
            day: zoned.day() as u8,
            weekday: zoned.weekday().to_monday_zero_offset() as u8,
            hour: zoned.hour() as u8,
            minute: zoned.minute() as u8,
        })
    }
}

impl PartialEq for TimeZone {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for TimeZone {}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;

    fn at(text: &str) -> i64 {
        text.parse::<Timestamp>()
            .expect("an instant")
            .as_millisecond()
    }

    #[test]
    fn a_day_in_bogota_begins_at_five_in_the_morning_utc() {
        let bogota = TimeZone::named("America/Bogota").expect("a zone");
        let late = at("2026-11-28T03:00:00Z");
        assert_eq!(bogota.day_of(late), at("2026-11-27T05:00:00Z"));
        assert_eq!(bogota.next_day(late), at("2026-11-28T05:00:00Z"));
        let moment = bogota.moment(late).expect("a moment");
        assert_eq!((moment.day, moment.hour), (27, 22));
    }

    #[test]
    fn the_day_the_clocks_go_forward_is_twenty_three_hours_long() {
        let madrid = TimeZone::named("Europe/Madrid").expect("a zone");
        let day = madrid.day_of(at("2026-03-29T12:00:00Z"));
        assert_eq!(madrid.next_day(day) - day, 23 * HOUR);
        assert_eq!(madrid.previous_day(madrid.next_day(day)), day);
    }

    #[test]
    fn a_zone_this_binary_does_not_know_is_no_zone() {
        assert!(TimeZone::named("Mars/Olympus_Mons").is_none());
        assert!(TimeZone::named("../../etc/passwd").is_none());
    }

    #[test]
    fn without_a_reader_the_days_are_utc_and_say_so() {
        let utc = TimeZone::utc();
        assert!(utc.reader_unknown());
        assert_eq!(
            utc.day_of(at("2026-11-27T23:59:00Z")),
            at("2026-11-27T00:00:00Z")
        );
    }
}
