//! One day up close: what it came to, what happened on it, and its charts
//! on one clock.

use hypertext::prelude::*;
use hypertext::Raw;

use crate::domain::entities::copy_metric::BUCKET_MS;
use crate::domain::entities::service_event::{EventKind, Severity};
use crate::domain::policies::stability::Row;
use crate::features::stability::see_service_stability_request::day_param;
use crate::features::stability::service_stability_response::{
    Chart, EventLine, ServiceStabilityResponse,
};
use crate::features::stability::stability_chart_component::Plot;
use crate::features::stability::texts::{self, t};
use crate::shell::components::declaration;

const DEPLOY: &str = "M2 12a10 10 0 1 0 20 0a10 10 0 1 0-20 0M16 12l-4-4-4 4M12 16V8";
const CONFIG: &str = "M21 4h-7M10 4H3M21 12h-9M8 12H3M21 20h-5M12 20H3M14 2v4M8 10v4M16 18v4";
const OOM: &str =
    "m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3ZM12 9v4M12 17h.01";
const CRASH: &str = "M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8M3 3v5h5";
const NODE: &str = "m19 5 3-3m-20 20 3-3M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4ZM7.5 13.5 10 11m.5 5.5L13 14m-1-8 6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z";
const THROTTLE: &str = "m12 14 4-4M3.34 19a10 10 0 1 1 17.32 0";
const MEMORY: &str = "M22 7l-8.5 8.5-5-5L2 17M16 7h6v6";
const BACKUP: &str = "M3 3h18a1 1 0 0 1 1 1v3a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1ZM4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8M10 12h4";
const DISK: &str = "M22 12H2M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11zM6 16h.01M10 16h.01";
const UNFOLLOW: &str = "m18.84 12.25 1.72-1.71h-.02a5.004 5.004 0 0 0-.12-7.07 5.006 5.006 0 0 0-6.95 0l-1.72 1.71M5.17 11.75l-1.71 1.71a5.004 5.004 0 0 0 .12 7.07 5.006 5.006 0 0 0 6.95 0l1.71-1.71M8 2v3M2 8h3M16 19v3M19 16h3";
const STOP: &str = "M6 6h12v12H6z";
const START: &str = "M6 3l14 9-14 9z";
const CLOSE: &str = "M18 6 6 18M6 6l12 12";

pub fn icon(kind: EventKind) -> Raw<String> {
    let path = match kind {
        EventKind::Deployed => DEPLOY,
        EventKind::EnvChanged => CONFIG,
        EventKind::OomKilled => OOM,
        EventKind::Crashed | EventKind::Down => CRASH,
        EventKind::Unmeasured => NODE,
        EventKind::Throttled => THROTTLE,
        EventKind::MemoryHigh => MEMORY,
        EventKind::BackedUp | EventKind::BackupFailed => BACKUP,
        EventKind::DiskHigh => DISK,
        EventKind::Unfollowed => UNFOLLOW,
        EventKind::Stopped => STOP,
        EventKind::Started => START,
    };
    glyph(path)
}

fn glyph(path: &str) -> Raw<String> {
    Raw::dangerously_create(format!(
        r#"<svg class="st-ico" viewBox="0 0 24 24" aria-hidden="true"><path d="{path}"/></svg>"#
    ))
}

pub fn severity(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Serious => "serious",
        Severity::Critical => "critical",
    }
}

pub fn who(event: &EventLine, copies: usize) -> String {
    match event.slot {
        Some(slot) if copies > 1 => texts::copy(slot, event.node.as_deref()),
        Some(_) => event.node.clone().unwrap_or_default(),
        None if copies > 1 => t("Every copy").to_string(),
        None => String::new(),
    }
}

fn place(response: &ServiceStabilityResponse, at: i64) -> f64 {
    let span = response.span as i64 * BUCKET_MS;
    ((at - response.start) as f64 / span as f64 * 100.0).clamp(0.0, 100.0)
}

struct Marker {
    class: String,
    x: f64,
    href: String,
    label: String,
    glyph: Raw<String>,
    count: usize,
}

struct Item {
    glyph: Raw<String>,
    title: String,
    when: String,
    detail: Option<(String, bool)>,
}

struct Card {
    x: f64,
    pinned: bool,
    items: Vec<Item>,
}

struct Group<'a> {
    x: f64,
    events: Vec<&'a EventLine>,
}

impl Group<'_> {
    fn worst(&self) -> &EventLine {
        self.events
            .iter()
            .max_by_key(|event| event.severity)
            .copied()
            .unwrap_or(self.events[0])
    }
}

fn groups(response: &ServiceStabilityResponse) -> Vec<Group<'_>> {
    let mut groups: Vec<Group> = Vec::new();
    for event in &response.day_events {
        let x = place(response, event.at.max(response.selected));
        match groups.last_mut() {
            Some(group) if x - group.x < 1.8 => group.events.push(event),
            _ => groups.push(Group {
                x,
                events: vec![event],
            }),
        }
    }
    groups
}

fn items(items: &[Item]) -> impl Renderable + '_ {
    rsx! {
        @for item in items {
            <div class="st-evitem">
                (&item.glyph)
                <div><div class="st-evtitle">(&item.title)</div><div class="st-evmeta">(&item.when)</div></div>
                @match &item.detail {
                    Some((text, true)) => { <div class="st-evdetail mono">(text)</div> }
                    Some((text, false)) => { <div class="st-evdetail">(text)</div> }
                    None => {}
                }
            </div>
        }
    }
}

pub fn day<'a>(
    response: &'a ServiceStabilityResponse,
    plots: &'a [Plot],
    base: &'a str,
) -> impl Renderable + 'a {
    let selected = response.selected;
    let zone = &response.zone;
    let link = |day: i64, event: Option<&str>| match event {
        Some(id) => format!(
            "{base}?days={}&day={}&event={id}#day",
            response.days,
            day_param(zone, day)
        ),
        None => format!(
            "{base}?days={}&day={}#day",
            response.days,
            day_param(zone, day)
        ),
    };
    let first = response
        .strip
        .first()
        .map(|day| day.start)
        .unwrap_or(selected)
        .max(response.since.unwrap_or(selected));
    let previous = (selected > first).then(|| link(zone.previous_day(selected), None));
    let next = (selected < response.today).then(|| link(zone.next_day(selected), None));
    let mut title = texts::full_day(selected);
    if selected == response.today {
        title.push_str(&format!(" · {}", t("today")));
    }
    if zone.reader_unknown() {
        title.push_str(" · UTC");
    }
    let stats = response
        .strip
        .iter()
        .find(|day| day.start == selected)
        .map(|day| day.stats.clone())
        .unwrap_or_default();
    let facts = texts::facts(
        &stats,
        response.copies.len(),
        response.charts.contains(&Chart::Disk),
        response.rows.contains(&Row::Backups),
    );
    let copies = response.copies.len();
    let keys: Vec<(String, String)> = response
        .copies
        .iter()
        .enumerate()
        .map(|(index, copy)| {
            let colour = ["c1", "c2", "c3", "c4"].get(index).copied().unwrap_or("c0");
            (
                format!("st-line {colour}"),
                texts::copy(copy.slot, copy.node.as_deref()),
            )
        })
        .collect();
    let grouped = groups(response);
    let pinned = response.pinned.as_deref();
    let markers: Vec<Marker> = grouped
        .iter()
        .map(|group| {
            let worst = group.worst();
            let on = group
                .events
                .iter()
                .any(|event| Some(event.id.as_str()) == pinned);
            let label = group
                .events
                .iter()
                .map(|event| format!("{} {}", texts::clock(event.at), texts::title(event)))
                .collect::<Vec<_>>()
                .join("; ");
            let href = match on {
                true => link(selected, None),
                false => link(selected, Some(&group.events[0].id)),
            };
            let class = match on {
                true => format!("st-mk {} on", severity(worst.severity)),
                false => format!("st-mk {}", severity(worst.severity)),
            };
            Marker {
                class,
                x: group.x,
                href,
                label,
                glyph: icon(worst.kind),
                count: group.events.len(),
            }
        })
        .collect();
    let cards: Vec<Card> = grouped
        .iter()
        .map(|group| {
            let on = group
                .events
                .iter()
                .any(|event| Some(event.id.as_str()) == pinned);
            let items = group
                .events
                .iter()
                .map(|event| {
                    let mut when = texts::clock(event.at);
                    if let Some(until) = event.until {
                        when.push_str(&format!("–{}", texts::clock(until)));
                    }
                    let who = who(event, copies);
                    if !who.is_empty() {
                        when.push_str(&format!(" · {who}"));
                    }
                    Item {
                        glyph: icon(event.kind),
                        title: texts::title(event),
                        when,
                        detail: texts::detail(event),
                    }
                })
                .collect();
            Card {
                x: group.x,
                pinned: on,
                items,
            }
        })
        .collect();
    let regions: Vec<(String, f64, f64)> = response
        .day_events
        .iter()
        .filter_map(|event| {
            let until = event.until?;
            let start = place(response, event.at);
            let width = (place(response, until) - start).max(0.4);
            Some((
                format!("st-region {}", severity(event.severity)),
                start,
                width,
            ))
        })
        .collect();
    let marks: Vec<(String, f64)> = response
        .day_events
        .iter()
        .filter(|event| event.until.is_none())
        .map(|event| {
            (
                format!("st-vline {}", severity(event.severity)),
                place(response, event.at),
            )
        })
        .collect();
    let hours: Vec<(f64, String)> = (3..24)
        .step_by(3)
        .filter_map(|hour| {
            let at = zone.hour_on(selected, hour)?;
            Some((place(response, at), format!("{hour:02}:00")))
        })
        .collect();
    let close = link(selected, None);
    rsx! {
        <section class="card st-card" id="day">
            <div class="split">
                <h2>(title)</h2>
                <div class="row">
                    @if let Some(href) = &previous {
                        <a class="btn btn-secondary btn-sm" href=(href)>(t("Previous day"))</a>
                    } @else {
                        <span class="btn btn-secondary btn-sm st-off" aria-disabled="true">(t("Previous day"))</span>
                    }
                    @if let Some(href) = &next {
                        <a class="btn btn-secondary btn-sm" href=(href)>(t("Next day"))</a>
                    } @else {
                        <span class="btn btn-secondary btn-sm st-off" aria-disabled="true">(t("Next day"))</span>
                    }
                </div>
            </div>
            <div class="st-facts">
                @for fact in &facts {
                    <span><b>(&fact.value)</b>" "(&fact.label)</span>
                }
            </div>
            <div class="st-legend st-indent">
                @for (class, label) in &keys {
                    <span><i class=(class)></i>(label)</span>
                }
                <span><i class="st-dash"></i>(t("Ceiling"))</span>
            </div>
            <div class="st-lane" data-st-lane>
                <span class="st-lane-label">(t("Events"))</span>
                <div class="st-lane-base"></div>
                @for (index, marker) in markers.iter().enumerate() {
                    <a class=(&marker.class) style=(declaration::left(marker.x)) href=(&marker.href)
                       aria-label=(&marker.label) title=(&marker.label) data-st-group=(index)>
                        (&marker.glyph)
                        @if marker.count > 1 {
                            <span class="st-n">(marker.count)</span>
                        }
                    </a>
                }
                @for (index, card) in cards.iter().enumerate() {
                    @if card.pinned {
                        <div class="st-evcard" style=(declaration::beside(card.x, -14)) data-st-card=(index) data-st-pinned>
                            (items(&card.items))
                            <div class="st-evactions"><a href=(&close) aria-label=(t("Close"))>(glyph(CLOSE))</a></div>
                        </div>
                    } @else {
                        <div class="st-evcard" style=(declaration::beside(card.x, -14)) data-st-card=(index) hidden>
                            (items(&card.items))
                        </div>
                    }
                }
            </div>
            <div class="st-charts" data-st-charts>
                @for (index, plot) in plots.iter().enumerate() {
                    <div class="st-chart">
                        <div class="st-chart-head">
                            <span class="st-chart-title">(plot.title)</span>
                            <span class="st-chart-sum">(&plot.summary)</span>
                        </div>
                        <div class="st-chart-body">
                            <div class="st-y">
                                @for (top, label) in &plot.ticks {
                                    <span style=(declaration::top(*top))>(label)</span>
                                }
                            </div>
                            <div class="st-plot" data-st-plot=(index)>
                                @for (class, start, width) in &regions {
                                    <div class=(class) style=(declaration::span(*start, *width))></div>
                                }
                                (Raw::dangerously_create(plot.svg.clone()))
                                @if let Some((up, down)) = plot.mirrored {
                                    <span class="st-inlabel st-top">(up)</span>
                                    <span class="st-inlabel st-bottom">(down)</span>
                                }
                                @for (class, x) in &marks {
                                    <div class=(class) style=(declaration::left(*x))></div>
                                }
                            </div>
                        </div>
                    </div>
                }
                <div class="st-x">
                    @for (left, label) in &hours {
                        <span style=(declaration::left(*left))>(label)</span>
                    }
                </div>
            </div>
        </section>
    }
}
