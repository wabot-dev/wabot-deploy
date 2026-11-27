//! What the stability page says. Sorted by the English.

use crate::domain::entities::service_event::EventKind;
use crate::domain::policies::stability::{Reason, Row, State, Stop};
use crate::domain::values::language::Language;
use crate::domain::values::time_zone::{Moment, TimeZone};
use crate::features::stability::service_stability_response::EventLine;

pub(crate) const TABLE: &[(&str, &str)] = &[
    ("Availability", "Disponibilidad"),
    ("Backup failed", "Backup fallido"),
    ("Backups", "Backups"),
    ("Ceiling", "Techo"),
    ("Close", "Cerrar"),
    ("Day by day", "Día a día"),
    ("Deploys", "Despliegues"),
    ("Detail", "Detalle"),
    ("Disk I/O", "E/S de disco"),
    ("Event", "Evento"),
    ("Events", "Eventos"),
    ("Events in this period", "Eventos en este periodo"),
    ("Every copy", "Todas"),
    ("In", "Entrada"),
    ("Network", "Red"),
    ("Next day", "Día siguiente"),
    ("No data", "Sin datos"),
    ("No events.", "Ningún evento."),
    ("OOM kill", "OOM kill"),
    ("Out", "Salida"),
    ("Period", "Periodo"),
    ("Previous day", "Día anterior"),
    ("Read", "Lectura"),
    ("Stopped", "Detenido"),
    ("Variables changed", "Variables cambiadas"),
    ("Write", "Escritura"),
    ("attention", "atención"),
    ("ceiling", "techo"),
    ("following", "siguiendo"),
    ("good", "bien"),
    ("here", "aquí"),
    ("incident", "incidencia"),
    ("no data", "sin datos"),
    ("none", "ninguno"),
    ("off", "apagados"),
    ("today", "hoy"),
];

pub fn t(english: &'static str) -> &'static str {
    crate::shell::texts::language::translated(english, TABLE)
}

fn spanish() -> bool {
    crate::shell::texts::language::current() == Language::Es
}

thread_local! {
    static ZONE: std::cell::RefCell<TimeZone> = std::cell::RefCell::new(TimeZone::utc());
}

/// Every time this page prints, in the reader's zone: scoped around the
/// render, as the language is, rather than carried to every reason that
/// says when something happened.
pub fn zoned<T>(zone: &TimeZone, render: impl FnOnce() -> T) -> T {
    let before = ZONE.with(|current| current.replace(zone.clone()));
    let rendered = render();
    ZONE.with(|current| current.replace(before));
    rendered
}

fn in_zone(at: i64) -> Option<Moment> {
    ZONE.with(|zone| zone.borrow().moment(at))
}

pub fn decimal(value: f64, places: usize) -> String {
    let text = format!("{value:.places$}");
    match spanish() {
        true => text.replace('.', ","),
        false => text,
    }
}

pub fn bytes(value: u64) -> String {
    let text = crate::infra::node::memory::human(value);
    match spanish() {
        true => text.replace('.', ","),
        false => text,
    }
}

pub fn rate(bytes_per_second: u64) -> String {
    format!("{}/s", bytes(bytes_per_second))
}

pub fn millicores(value: u32) -> String {
    format!("{value}m")
}

pub fn clock(at: i64) -> String {
    in_zone(at)
        .map(|moment| format!("{:02}:{:02}", moment.hour, moment.minute))
        .unwrap_or_default()
}

const WEEKDAYS_EN: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const WEEKDAYS_ES: [&str; 7] = [
    "lunes",
    "martes",
    "miércoles",
    "jueves",
    "viernes",
    "sábado",
    "domingo",
];
const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const MONTHS_ES: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];

fn parts(at: i64) -> Option<(usize, u8, usize)> {
    let moment = in_zone(at)?;
    Some((
        moment.weekday as usize,
        moment.day,
        moment.month as usize - 1,
    ))
}

/// Friday 21 November.
pub fn full_day(at: i64) -> String {
    let Some((weekday, day, month)) = parts(at) else {
        return String::new();
    };
    match spanish() {
        true => format!("{} {day} de {}", WEEKDAYS_ES[weekday], MONTHS_ES[month]),
        false => format!("{} {day} {}", WEEKDAYS_EN[weekday], MONTHS_EN[month]),
    }
}

/// 21 Nov.
pub fn short_day(at: i64) -> String {
    let Some((_, day, month)) = parts(at) else {
        return String::new();
    };
    match spanish() {
        true => format!("{day} {}", &MONTHS_ES[month][..3]),
        false => format!("{day} {}", &MONTHS_EN[month][..3]),
    }
}

/// 21 Nov 14:05, the way the events table says when.
pub fn moment(at: i64) -> String {
    format!("{} {}", short_day(at), clock(at))
}

pub fn days(count: u32) -> String {
    match spanish() {
        true => format!("{count} días"),
        false => format!("{count} days"),
    }
}

pub fn row(row: Row) -> &'static str {
    match row {
        Row::Availability => t("Availability"),
        Row::Memory => t("Memory"),
        Row::Cpu => t("CPU"),
        Row::Network => t("Network"),
        Row::Disk => t("Disk"),
        Row::Backups => t("Backups"),
        Row::Replicas => t("Replicas"),
    }
}

pub fn state(state: State) -> &'static str {
    match state {
        State::Good => t("good"),
        State::Attention => t("attention"),
        State::Incident => t("incident"),
        State::NoData => t("no data"),
    }
}

/// The noun, which the shared table has as the verb.
pub fn copy_heading() -> &'static str {
    match spanish() {
        true => "Copia",
        false => "Copy",
    }
}

pub fn copy(slot: u32, node: Option<&str>) -> String {
    format!("{} {slot} · {}", copy_heading(), node.unwrap_or(t("here")))
}

fn whose(slot: Option<u32>) -> String {
    slot.map(|slot| format!("{} {slot} · ", copy_heading().to_lowercase()))
        .unwrap_or_default()
}

fn stops(stops: &[Stop]) -> String {
    stops
        .iter()
        .map(|stop| {
            let what = match (stop.oom, stop.code) {
                (true, _) => t("OOM kill").to_string(),
                (false, Some(code)) => format!("exit {code}"),
                (false, None) => t("Stopped").to_lowercase(),
            };
            let copy = stop
                .slot
                .map(|slot| format!(" ({} {slot})", copy_heading().to_lowercase()))
                .unwrap_or_default();
            format!("{what} {}{copy}", clock(stop.at))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn minutes_at_ceiling(minutes: u32) -> String {
    match spanish() {
        true => format!("{minutes} min en el techo"),
        false => format!("{minutes} min at its ceiling"),
    }
}

pub fn peak(what: String) -> String {
    match spanish() {
        true => format!("pico {what}"),
        false => format!("peak {what}"),
    }
}

pub fn full_in(days: u32) -> String {
    match spanish() {
        true => format!("lleno en ~{} días", days.max(1)),
        false => format!("full in ~{} days", days.max(1)),
    }
}

/// Why a day's cell is the colour it is, in a line.
pub fn why(reason: &Reason) -> String {
    let es = spanish();
    match reason {
        Reason::NoData => t("no data").to_string(),
        Reason::Available { percent } => {
            let places = if *percent >= 100.0 { 0 } else { 2 };
            format!("{} %", decimal(*percent, places))
        }
        Reason::Down { slot, minutes } => match es {
            true => format!("{}sin correr {minutes} min", whose(*slot)),
            false => format!("{}not running for {minutes} min", whose(*slot)),
        },
        Reason::Unmeasured { slot, minutes } => match es {
            true => format!("{}sin medidas {minutes} min", whose(*slot)),
            false => format!("{}no readings for {minutes} min", whose(*slot)),
        },
        Reason::Stopped { stops: list } | Reason::OomKilled { stops: list } => stops(list),
        Reason::MemoryPeak {
            bytes: value,
            percent,
        } => match percent {
            Some(percent) => format!("{} · {percent} %", peak(bytes(*value))),
            None => peak(bytes(*value)),
        },
        Reason::Throttled { minutes } => minutes_at_ceiling(*minutes),
        Reason::CpuPeak { millicores: value } => peak(millicores(*value)),
        Reason::Traffic { ratio } => match es {
            true => format!("tráfico ×{}", decimal(*ratio, 1)),
            false => format!("traffic ×{}", decimal(*ratio, 1)),
        },
        Reason::TrafficPeak { bytes_per_second } => peak(rate(*bytes_per_second)),
        Reason::Disk {
            bytes: value,
            percent,
            full_in_days,
        } => {
            let mut text = bytes(*value);
            if let Some(percent) = percent {
                text.push_str(&format!(" · {percent} %"));
            }
            if let Some(days) = full_in_days.filter(|days| *days < 14) {
                text.push_str(&format!(" · {}", full_in(days)));
            }
            text
        }
        Reason::BackedUp { at, detail } => match detail {
            Some(detail) => format!("{} · {detail}", clock(*at)),
            None => clock(*at),
        },
        Reason::BackupRetried { failed_at, done_at } => match es {
            true => format!("{} falló, {} hecho", clock(*failed_at), clock(*done_at)),
            false => format!("{} failed, {} done", clock(*failed_at), clock(*done_at)),
        },
        Reason::BackupMissing { failed, .. } => match failed {
            Some((at, Some(reason))) => match es {
                true => format!("{} falló · {reason}", clock(*at)),
                false => format!("{} failed · {reason}", clock(*at)),
            },
            Some((at, None)) => match es {
                true => format!("{} falló", clock(*at)),
                false => format!("{} failed", clock(*at)),
            },
            None => t("none").to_string(),
        },
        Reason::BackupNotDue { last_ok } => match es {
            true => format!("el último, {}", moment(*last_ok)),
            false => format!("last one {}", moment(*last_ok)),
        },
        Reason::BackupsOff => t("off").to_string(),
        Reason::Following => t("following").to_string(),
        Reason::Unfollowed { minutes } => match es {
            true => format!("standby sin seguir {minutes} min"),
            false => format!("standby not following for {minutes} min"),
        },
    }
}

fn span_minutes(event: &EventLine) -> Option<i64> {
    event
        .code
        .or(event.until.map(|until| (until - event.at) / 60_000))
}

/// A deployment is known by its tag, never by its registry.
pub fn tag(reference: &str) -> String {
    let last = reference.rsplit('/').next().unwrap_or(reference);
    let last = last.split('@').next().unwrap_or(last);
    match last.split_once(':') {
        Some((_, tag)) => tag.to_string(),
        None => last.to_string(),
    }
}

/// What an event is called where it is listed.
pub fn title(event: &EventLine) -> String {
    let es = spanish();
    let minutes = span_minutes(event).unwrap_or(0);
    match event.kind {
        EventKind::Deployed => {
            let tag = tag(event.detail.as_deref().unwrap_or(""));
            match es {
                true => format!("Despliegue {tag}"),
                false => format!("Deployed {tag}"),
            }
        }
        EventKind::Stopped => t("Stopped").to_string(),
        EventKind::Started => match es {
            true => "Arrancado".to_string(),
            false => "Started".to_string(),
        },
        EventKind::Crashed => match (es, event.code) {
            (true, Some(code)) => format!("Se detuvo con exit {code}"),
            (false, Some(code)) => format!("Stopped with exit {code}"),
            (true, None) => "Se detuvo sola".to_string(),
            (false, None) => "Stopped on its own".to_string(),
        },
        EventKind::OomKilled => t("OOM kill").to_string(),
        EventKind::BackedUp => t("Backup").to_string(),
        EventKind::BackupFailed => t("Backup failed").to_string(),
        EventKind::Unfollowed => match (es, event.until) {
            (true, Some(_)) => format!("Standby sin seguir {minutes} min"),
            (false, Some(_)) => format!("Standby not following for {minutes} min"),
            (true, None) => "Standby sin seguir".to_string(),
            (false, None) => "Standby not following".to_string(),
        },
        EventKind::Unmeasured => match (es, event.node.as_deref()) {
            (true, Some(node)) => format!("{node} sin reportar {minutes} min"),
            (false, Some(node)) => format!("{node} not reporting for {minutes} min"),
            (true, None) => format!("Sin medidas {minutes} min"),
            (false, None) => format!("No readings for {minutes} min"),
        },
        EventKind::Down => match es {
            true => format!("Sin correr {minutes} min"),
            false => format!("Not running for {minutes} min"),
        },
        EventKind::Throttled => match es {
            true => format!("CPU en su techo {minutes} min"),
            false => format!("CPU at its ceiling for {minutes} min"),
        },
        EventKind::MemoryHigh => match es {
            true => format!("Memoria al {} % del techo", event.code.unwrap_or(0)),
            false => format!("Memory at {} % of its ceiling", event.code.unwrap_or(0)),
        },
        EventKind::DiskHigh => match es {
            true => format!("Disco al {} % del techo", event.code.unwrap_or(0)),
            false => format!("Disk at {} % of its ceiling", event.code.unwrap_or(0)),
        },
        EventKind::EnvChanged => t("Variables changed").to_string(),
    }
}

/// The line under an event's title, and whether it is machine text.
pub fn detail(event: &EventLine) -> Option<(String, bool)> {
    let detail = event.detail.as_deref()?;
    Some(match event.kind {
        EventKind::MemoryHigh | EventKind::DiskHigh => (
            detail
                .parse::<u64>()
                .map(bytes)
                .unwrap_or_else(|_| detail.to_string()),
            false,
        ),
        EventKind::BackedUp | EventKind::BackupFailed => {
            (detail.to_string(), event.kind == EventKind::BackupFailed)
        }
        _ => (detail.to_string(), true),
    })
}

/// The day's facts above its charts, in the order the page shows them.
pub struct Fact {
    pub value: String,
    pub label: String,
}

pub fn facts(
    stats: &crate::domain::policies::stability::DayStats,
    copies: usize,
    disk: bool,
    backups: bool,
) -> Vec<Fact> {
    let es = spanish();
    let fact = |value: String, label: &str| Fact {
        value,
        label: label.to_string(),
    };
    let plural =
        |value: u32, en: (&'static str, &'static str), sp: (&'static str, &'static str)| {
            let (one, many) = if es { sp } else { en };
            fact(value.to_string(), if value == 1 { one } else { many })
        };
    let mut facts = vec![
        plural(
            stats.deploys,
            ("deploy", "deploys"),
            ("despliegue", "despliegues"),
        ),
        plural(
            stats.oom_kills,
            ("OOM kill", "OOM kills"),
            ("OOM kill", "OOM kills"),
        ),
        plural(stats.crashes, ("crash", "crashes"), ("caída", "caídas")),
        fact(
            format!("{} min", stats.throttled_minutes),
            if es {
                "con la CPU en el techo"
            } else {
                "with the CPU at its ceiling"
            },
        ),
        fact(
            format!("{} min", stats.missing_minutes),
            match (es, copies > 1) {
                (true, true) => "sin medidas de alguna copia",
                (true, false) => "sin medidas",
                (false, true) => "without readings from a copy",
                (false, false) => "without readings",
            },
        ),
    ];
    if disk {
        if let Some(change) = stats.disk_change {
            let sign = if change < 0 { "−" } else { "+" };
            facts.push(fact(
                format!("{sign}{}", bytes(change.unsigned_abs())),
                if es { "en disco" } else { "on disk" },
            ));
        }
    }
    if backups {
        facts.push(plural(
            stats.backups_failed,
            ("failed backup", "failed backups"),
            ("backup fallido", "backups fallidos"),
        ));
    }
    facts
}

pub fn not_asked(node: &str, reason: &str) -> String {
    match spanish() {
        true => format!("No se pudo preguntar a {node}: {reason}"),
        false => format!("{node} could not be asked: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deployment_is_labelled_by_its_tag_not_its_registry() {
        assert_eq!(tag("node:5000/demo/api:v19"), "v19");
        assert_eq!(tag("node.example/demo/api@sha256:abc"), "api");
        assert_eq!(tag("docker.io/traefik/whoami:v1.10"), "v1.10");
    }

    #[test]
    fn a_time_is_printed_on_the_readers_clock_and_only_while_the_render_lasts() {
        let late = 1_795_316_400_000;
        let bogota = TimeZone::named("America/Bogota").expect("a zone");
        let (clock_there, day_there) = zoned(&bogota, || (clock(late), short_day(late)));
        assert_eq!(
            (clock_there.as_str(), day_there.as_str()),
            ("22:00", "21 Nov")
        );
        assert_eq!(
            (clock(late).as_str(), short_day(late).as_str()),
            ("03:00", "22 Nov")
        );
    }
}
