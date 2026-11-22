//! How a day went for a service, read from what its copies measured and
//! what happened to it (`docs/stability.md`).
//!
//! Days are UTC, like every time the console prints.

use std::collections::BTreeMap;

use crate::domain::entities::copy_metric::{CopyMetric, BUCKET_MS};
use crate::domain::entities::service_event::{EventKind, ServiceEvent};

pub const DAY_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;

/// A bucket is judged once the sampler has certainly written it: it closes
/// the ten minutes on its first reading after them, a minute later at most.
const SETTLED_MS: i64 = 2 * BUCKET_MS;
const MISSING_MINUTES: u32 = 15;
const THROTTLED_ATTENTION: u32 = 10;
const THROTTLED_INCIDENT: u32 = 120;
const MEMORY_ATTENTION: u32 = 90;
const DISK_ATTENTION: u32 = 85;
const DISK_INCIDENT: u32 = 95;
const FULL_SOON_DAYS: u32 = 14;
const TRAFFIC_ATTENTION: f64 = 1.8;
const BACKUP_SLACK_MS: i64 = 6 * 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Row {
    Availability,
    Memory,
    Cpu,
    Network,
    Disk,
    Backups,
    Replicas,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Good,
    Attention,
    Incident,
    NoData,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    pub at: i64,
    pub slot: Option<u32>,
    pub oom: bool,
    pub code: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reason {
    NoData,
    Available {
        percent: f64,
    },
    Down {
        slot: Option<u32>,
        minutes: u32,
    },
    Unmeasured {
        slot: Option<u32>,
        minutes: u32,
    },
    Stopped {
        stops: Vec<Stop>,
    },
    MemoryPeak {
        bytes: u64,
        percent: Option<u32>,
    },
    OomKilled {
        stops: Vec<Stop>,
    },
    Throttled {
        minutes: u32,
    },
    CpuPeak {
        millicores: u32,
    },
    Traffic {
        ratio: f64,
    },
    TrafficPeak {
        bytes_per_second: u64,
    },
    Disk {
        bytes: u64,
        percent: Option<u32>,
        full_in_days: Option<u32>,
    },
    BackedUp {
        at: i64,
        detail: Option<String>,
    },
    BackupRetried {
        failed_at: i64,
        done_at: i64,
    },
    BackupMissing {
        failed: Option<(i64, Option<String>)>,
        last_ok: Option<i64>,
    },
    BackupNotDue {
        last_ok: i64,
    },
    BackupsOff,
    Following,
    Unfollowed {
        minutes: u32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub state: State,
    pub reason: Reason,
}

impl Verdict {
    fn new(state: State, reason: Reason) -> Self {
        Self { state, reason }
    }

    fn none() -> Self {
        Self::new(State::NoData, Reason::NoData)
    }
}

pub struct Copy<'a> {
    pub slot: u32,
    pub container: &'a str,
    pub metrics: &'a [CopyMetric],
    pub current: bool,
}

pub struct Input<'a> {
    pub now: i64,
    pub copies: &'a [Copy<'a>],
    pub events: &'a [ServiceEvent],
    pub rows: &'a [Row],
    pub backup_every: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DayStats {
    pub deploys: u32,
    pub oom_kills: u32,
    pub crashes: u32,
    pub throttled_minutes: u32,
    pub missing_minutes: u32,
    pub disk_change: Option<i64>,
    pub backups_failed: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub start: i64,
    pub verdicts: BTreeMap<Row, Verdict>,
    pub stats: DayStats,
}

pub fn day_of(at: i64) -> i64 {
    at - at.rem_euclid(DAY_MS)
}

/// The first moment anything was measured for this service: before it,
/// a day has no data rather than a gap.
pub fn since(copies: &[Copy]) -> Option<i64> {
    copies
        .iter()
        .filter_map(|copy| copy.metrics.first().map(|metric| metric.bucket))
        .min()
}

/// When the service was meant to be stopped, from its own events.
pub fn stopped(events: &[ServiceEvent], now: i64) -> Vec<(i64, i64)> {
    let mut spans = Vec::new();
    let mut open: Option<i64> = None;
    for event in events {
        match event.kind {
            EventKind::Stopped if open.is_none() => open = Some(event.at),
            EventKind::Started => {
                if let Some(from) = open.take() {
                    spans.push((from, event.at));
                }
            }
            _ => {}
        }
    }
    if let Some(from) = open {
        spans.push((from, now));
    }
    spans
}

fn inside(spans: &[(i64, i64)], from: i64, to: i64) -> bool {
    spans.iter().any(|(a, b)| *a <= from && to <= *b)
}

fn expected_buckets(copy: &Copy, now: i64, stops: &[(i64, i64)], from: i64, to: i64) -> Vec<i64> {
    let Some(first) = copy.metrics.first().map(|metric| metric.bucket) else {
        return Vec::new();
    };
    let end = match copy.current {
        true => now - SETTLED_MS,
        false => copy
            .metrics
            .last()
            .map(|m| m.bucket + BUCKET_MS)
            .unwrap_or(first),
    };
    let mut buckets = Vec::new();
    let mut bucket = from.max(first);
    bucket -= bucket.rem_euclid(BUCKET_MS);
    while bucket < to && bucket + BUCKET_MS <= end {
        if !inside(stops, bucket, bucket + BUCKET_MS) {
            buckets.push(bucket);
        }
        bucket += BUCKET_MS;
    }
    buckets
}

fn in_range(metrics: &[CopyMetric], from: i64, to: i64) -> impl Iterator<Item = &CopyMetric> {
    metrics
        .iter()
        .filter(move |metric| metric.bucket >= from && metric.bucket < to)
}

fn percent(value: u64, of: Option<u64>) -> Option<u32> {
    of.filter(|of| *of > 0)
        .map(|of| (value.saturating_mul(100) / of) as u32)
}

fn slot_of(input: &Input, container: Option<&str>) -> Option<u32> {
    if input.copies.len() < 2 {
        return None;
    }
    let container = container?;
    input
        .copies
        .iter()
        .find(|copy| copy.container == container)
        .map(|copy| copy.slot)
}

struct Measured {
    missing: Vec<(Option<u32>, u32, bool)>,
    memory: Option<(u64, Option<u32>)>,
    cpu_max: u32,
    throttled: u32,
    traffic_peak: Option<u64>,
    disk_peak: Option<(u64, Option<u32>)>,
    disk_first: Option<u64>,
    disk_last: Option<u64>,
    disk_limit: Option<u64>,
}

fn measure(input: &Input, stops: &[(i64, i64)], from: i64, to: i64) -> Measured {
    let mut measured = Measured {
        missing: Vec::new(),
        memory: None,
        cpu_max: 0,
        throttled: 0,
        traffic_peak: None,
        disk_peak: None,
        disk_first: None,
        disk_last: None,
        disk_limit: None,
    };
    let mut traffic: BTreeMap<i64, u64> = BTreeMap::new();
    let mut throttled: BTreeMap<i64, u32> = BTreeMap::new();
    let mut disk: BTreeMap<i64, u64> = BTreeMap::new();
    for copy in input.copies {
        let slot = (input.copies.len() > 1).then_some(copy.slot);
        let rows: BTreeMap<i64, &CopyMetric> = in_range(copy.metrics, from, to)
            .map(|metric| (metric.bucket, metric))
            .collect();
        let expected = expected_buckets(copy, input.now, stops, from, to);
        let unmeasured = expected.iter().filter(|b| !rows.contains_key(b)).count() as u32 * 10;
        let down: u32 = rows.values().map(|metric| metric.down_minutes()).sum();
        if unmeasured > 0 {
            measured.missing.push((slot, unmeasured, false));
        }
        if down > 0 {
            measured.missing.push((slot, down, true));
        }
        for metric in rows.values() {
            if let Some(bytes) = metric.memory_max {
                let share = percent(bytes, metric.memory_limit);
                let higher = match measured.memory {
                    None => true,
                    Some((old, old_share)) => match (share, old_share) {
                        (Some(new), Some(old)) => new > old,
                        _ => bytes > old,
                    },
                };
                if higher {
                    measured.memory = Some((bytes, share));
                }
            }
            measured.cpu_max = measured.cpu_max.max(metric.cpu_max.unwrap_or(0));
            let held = throttled.entry(metric.bucket).or_default();
            *held = (*held).max(metric.throttled.unwrap_or(0));
            if metric.running > 0 {
                *traffic.entry(metric.bucket).or_default() +=
                    metric.received.unwrap_or(0) + metric.sent.unwrap_or(0);
            }
            if let Some(bytes) = metric.disk {
                let share = percent(bytes, metric.disk_limit);
                let higher = match measured.disk_peak {
                    None => true,
                    Some((old, old_share)) => {
                        share.unwrap_or(0) > old_share.unwrap_or(0) || bytes > old
                    }
                };
                if higher {
                    measured.disk_peak = Some((bytes, share));
                }
                let top = disk.entry(metric.bucket).or_default();
                *top = (*top).max(bytes);
                if metric.disk_limit.is_some() {
                    measured.disk_limit = metric.disk_limit;
                }
            }
        }
    }
    measured.throttled = throttled.values().map(|seconds| seconds / 60).sum();
    measured.traffic_peak = traffic
        .values()
        .max()
        .map(|bytes| bytes / (BUCKET_MS as u64 / 1000));
    measured.disk_first = disk.values().next().copied();
    measured.disk_last = disk.values().last().copied();
    measured
}

/// Every day from `first` for `count` days, oldest first.
pub fn days(input: &Input, first: i64, count: usize) -> Vec<Day> {
    let first = day_of(first);
    let since = since(input.copies);
    let stops = stopped(input.events, input.now);
    let lookback = 7usize;
    let measured: Vec<Measured> = (0..count + lookback)
        .map(|index| {
            let from = first + (index as i64 - lookback as i64) * DAY_MS;
            measure(input, &stops, from, from + DAY_MS)
        })
        .collect();

    (0..count)
        .map(|index| {
            let from = first + index as i64 * DAY_MS;
            let to = from + DAY_MS;
            let here = &measured[index + lookback];
            let events: Vec<&ServiceEvent> = input
                .events
                .iter()
                .filter(|event| event.at >= from && event.at < to)
                .collect();
            let known = since.is_some_and(|since| since < to) && from <= input.now;
            let stats = DayStats {
                deploys: count_of(&events, EventKind::Deployed),
                oom_kills: count_of(&events, EventKind::OomKilled),
                crashes: count_of(&events, EventKind::Crashed),
                throttled_minutes: here.throttled,
                missing_minutes: here.missing.iter().map(|(_, m, _)| *m).max().unwrap_or(0),
                disk_change: match (here.disk_first, here.disk_last) {
                    (Some(a), Some(b)) => Some(b as i64 - a as i64),
                    _ => None,
                },
                backups_failed: count_of(&events, EventKind::BackupFailed),
            };
            let mut verdicts = BTreeMap::new();
            for row in input.rows {
                let verdict = match known {
                    false => Verdict::none(),
                    true => match row {
                        Row::Availability => availability(input, here, &events),
                        Row::Memory => memory(input, here, &events),
                        Row::Cpu => cpu(here),
                        Row::Network => {
                            let previous: Vec<u64> = measured[index..index + lookback]
                                .iter()
                                .filter_map(|day| day.traffic_peak)
                                .collect();
                            network(here, &previous)
                        }
                        Row::Disk => {
                            let week_ago = measured[index].disk_last;
                            disk(here, week_ago)
                        }
                        Row::Backups => backups(input, since, &events, to),
                        Row::Replicas => replicas(input, from, to),
                    },
                };
                verdicts.insert(*row, verdict);
            }
            Day {
                start: from,
                verdicts,
                stats,
            }
        })
        .collect()
}

/// The day a page opens on when nobody chose one: the latest that was not
/// good, because that is the question the page is opened to answer.
pub fn worth_opening(days: &[Day]) -> Option<i64> {
    days.iter()
        .rev()
        .find(|day| {
            day.verdicts
                .values()
                .any(|verdict| matches!(verdict.state, State::Attention | State::Incident))
        })
        .map(|day| day.start)
}

fn count_of(events: &[&ServiceEvent], kind: EventKind) -> u32 {
    events.iter().filter(|event| event.kind == kind).count() as u32
}

fn stops(input: &Input, events: &[&ServiceEvent], kinds: &[EventKind]) -> Vec<Stop> {
    events
        .iter()
        .filter(|event| kinds.contains(&event.kind))
        .map(|event| Stop {
            at: event.at,
            slot: slot_of(input, event.container.as_deref()),
            oom: event.kind == EventKind::OomKilled,
            code: event.code,
        })
        .collect()
}

fn availability(input: &Input, here: &Measured, events: &[&ServiceEvent]) -> Verdict {
    if let Some((slot, minutes, down)) = here
        .missing
        .iter()
        .filter(|(_, minutes, _)| *minutes >= MISSING_MINUTES)
        .max_by_key(|(_, minutes, down)| (*down, *minutes))
    {
        return Verdict::new(
            State::Incident,
            match down {
                true => Reason::Down {
                    slot: *slot,
                    minutes: *minutes,
                },
                false => Reason::Unmeasured {
                    slot: *slot,
                    minutes: *minutes,
                },
            },
        );
    }
    let stopped = stops(input, events, &[EventKind::OomKilled, EventKind::Crashed]);
    if !stopped.is_empty() {
        return Verdict::new(State::Attention, Reason::Stopped { stops: stopped });
    }
    let lost: u32 = here.missing.iter().map(|(_, minutes, _)| *minutes).sum();
    let copies = input
        .copies
        .iter()
        .filter(|copy| !copy.metrics.is_empty())
        .count()
        .max(1);
    let percent = 100.0 * (1.0 - f64::from(lost) / (1440.0 * copies as f64));
    Verdict::new(
        State::Good,
        Reason::Available {
            percent: percent.max(0.0),
        },
    )
}

fn memory(input: &Input, here: &Measured, events: &[&ServiceEvent]) -> Verdict {
    let killed = stops(input, events, &[EventKind::OomKilled]);
    if !killed.is_empty() {
        return Verdict::new(State::Incident, Reason::OomKilled { stops: killed });
    }
    match here.memory {
        None => Verdict::none(),
        Some((bytes, percent)) => Verdict::new(
            match percent.is_some_and(|p| p >= MEMORY_ATTENTION) {
                true => State::Attention,
                false => State::Good,
            },
            Reason::MemoryPeak { bytes, percent },
        ),
    }
}

fn cpu(here: &Measured) -> Verdict {
    if here.throttled >= THROTTLED_INCIDENT {
        return Verdict::new(
            State::Incident,
            Reason::Throttled {
                minutes: here.throttled,
            },
        );
    }
    if here.throttled >= THROTTLED_ATTENTION {
        return Verdict::new(
            State::Attention,
            Reason::Throttled {
                minutes: here.throttled,
            },
        );
    }
    match here.memory.is_some() || here.cpu_max > 0 {
        true => Verdict::new(
            State::Good,
            Reason::CpuPeak {
                millicores: here.cpu_max,
            },
        ),
        false => Verdict::none(),
    }
}

fn network(here: &Measured, previous: &[u64]) -> Verdict {
    let Some(peak) = here.traffic_peak else {
        return Verdict::none();
    };
    let mut sorted = previous.to_vec();
    sorted.sort_unstable();
    if let Some(median) = sorted.get(sorted.len() / 2).copied().filter(|m| *m > 0) {
        let ratio = peak as f64 / median as f64;
        if ratio >= TRAFFIC_ATTENTION {
            return Verdict::new(State::Attention, Reason::Traffic { ratio });
        }
    }
    Verdict::new(
        State::Good,
        Reason::TrafficPeak {
            bytes_per_second: peak,
        },
    )
}

fn disk(here: &Measured, week_ago: Option<u64>) -> Verdict {
    let Some((bytes, percent)) = here.disk_peak else {
        return Verdict::none();
    };
    let full_in_days = match (here.disk_last, week_ago, here.disk_limit) {
        (Some(last), Some(before), Some(limit)) if last > before => {
            let rate = (last - before) as f64 / 7.0;
            Some((limit.saturating_sub(last) as f64 / rate).round() as u32)
        }
        _ => None,
    };
    let share = percent.unwrap_or(0);
    let state = if share >= DISK_INCIDENT {
        State::Incident
    } else if share >= DISK_ATTENTION || full_in_days.is_some_and(|d| d < FULL_SOON_DAYS) {
        State::Attention
    } else {
        State::Good
    };
    Verdict::new(
        state,
        Reason::Disk {
            bytes,
            percent,
            full_in_days: full_in_days.filter(|d| *d < 60),
        },
    )
}

fn backups(input: &Input, since: Option<i64>, events: &[&ServiceEvent], to: i64) -> Verdict {
    let Some(every) = input.backup_every else {
        return Verdict::new(State::NoData, Reason::BackupsOff);
    };
    let window = every + BACKUP_SLACK_MS;
    let done = events
        .iter()
        .find(|event| event.kind == EventKind::BackedUp);
    let failed = events
        .iter()
        .find(|event| event.kind == EventKind::BackupFailed);
    let last_ok = input
        .events
        .iter()
        .filter(|event| event.kind == EventKind::BackedUp && event.at < to)
        .map(|event| event.at)
        .max();
    match (done, failed) {
        (Some(done), Some(failed)) => Verdict::new(
            State::Attention,
            Reason::BackupRetried {
                failed_at: failed.at,
                done_at: done.at,
            },
        ),
        (Some(done), None) => Verdict::new(
            State::Good,
            Reason::BackedUp {
                at: done.at,
                detail: done.detail.clone(),
            },
        ),
        (None, failed) => {
            let end = to.min(input.now);
            let recent = last_ok.filter(|ok| end - ok <= window);
            let young = since.is_none_or(|since| end - since <= window);
            match (recent, failed) {
                (Some(ok), None) => Verdict::new(State::Good, Reason::BackupNotDue { last_ok: ok }),
                (None, None) if young => Verdict::none(),
                (Some(_), Some(failed)) => Verdict::new(
                    State::Attention,
                    Reason::BackupMissing {
                        failed: Some((failed.at, failed.detail.clone())),
                        last_ok,
                    },
                ),
                (None, failed) => Verdict::new(
                    State::Incident,
                    Reason::BackupMissing {
                        failed: failed.map(|f| (f.at, f.detail.clone())),
                        last_ok,
                    },
                ),
            }
        }
    }
}

fn replicas(input: &Input, from: i64, to: i64) -> Verdict {
    let minutes: i64 = input
        .events
        .iter()
        .filter(|event| event.kind == EventKind::Unfollowed)
        .map(|event| {
            let until = event.until.unwrap_or(input.now);
            (until.min(to) - event.at.max(from)).max(0) / MINUTE_MS
        })
        .sum();
    let minutes = minutes as u32;
    match minutes {
        0 => Verdict::new(State::Good, Reason::Following),
        m if m >= MISSING_MINUTES => {
            Verdict::new(State::Incident, Reason::Unfollowed { minutes: m })
        }
        m => Verdict::new(State::Attention, Reason::Unfollowed { minutes: m }),
    }
}

/// What the measurements say happened, as events beside the stored ones:
/// a copy that went unmeasured or down for a quarter of an hour, the CPU
/// held at its ceiling, memory past nine tenths of its ceiling, a disk
/// past 85 % and 95 % of its own. `code` carries the minutes or the
/// percentage, and `detail` the bytes, as digits for the page to format.
pub fn derived(input: &Input, from: i64, to: i64) -> Vec<ServiceEvent> {
    let stops = stopped(input.events, input.now);
    let mut found = Vec::new();
    for copy in input.copies {
        let rows: BTreeMap<i64, &CopyMetric> = in_range(copy.metrics, from, to)
            .map(|metric| (metric.bucket, metric))
            .collect();
        let expected = expected_buckets(copy, input.now, &stops, from, to);
        runs(
            expected.iter().filter(|b| !rows.contains_key(b)).copied(),
            |start, end, buckets| {
                let minutes = buckets as i64 * 10;
                (minutes >= i64::from(MISSING_MINUTES)).then(|| {
                    event(
                        copy,
                        EventKind::Unmeasured,
                        start,
                        Some(end),
                        Some(minutes),
                        None,
                    )
                })
            },
            &mut found,
        );
        let mut down_run: Option<(i64, i64, u32)> = None;
        let flush = |run: Option<(i64, i64, u32)>, found: &mut Vec<ServiceEvent>| {
            if let Some((start, end, minutes)) = run {
                if minutes >= MISSING_MINUTES {
                    found.push(event(
                        copy,
                        EventKind::Down,
                        start,
                        Some(end),
                        Some(i64::from(minutes)),
                        None,
                    ));
                }
            }
        };
        for metric in rows.values() {
            let down = metric.down_minutes();
            down_run = match (down_run, down) {
                (run, 0) => {
                    flush(run, &mut found);
                    None
                }
                (Some((start, end, minutes)), down) if end == metric.bucket => {
                    Some((start, metric.bucket + BUCKET_MS, minutes + down))
                }
                (run, down) => {
                    flush(run, &mut found);
                    Some((metric.bucket, metric.bucket + BUCKET_MS, down))
                }
            };
        }
        flush(down_run, &mut found);

        let mut high: Option<(i64, u64, u32)> = None;
        let mut armed = [true, true];
        let mut previous: Option<i64> = None;
        for metric in rows.values() {
            if let (Some(bytes), Some(share)) = (
                metric.memory_max,
                metric
                    .memory_max
                    .and_then(|b| percent(b, metric.memory_limit)),
            ) {
                let continuing = previous == Some(metric.bucket - BUCKET_MS);
                if share >= MEMORY_ATTENTION {
                    high = match high {
                        Some((at, top, peak)) if continuing => match share > peak {
                            true => Some((metric.bucket, bytes, share)),
                            false => Some((at, top, peak)),
                        },
                        other => {
                            if let Some((at, top, peak)) = other {
                                found.push(event(
                                    copy,
                                    EventKind::MemoryHigh,
                                    at,
                                    None,
                                    Some(i64::from(peak)),
                                    Some(top),
                                ));
                            }
                            Some((metric.bucket, bytes, share))
                        }
                    };
                } else if let Some((at, top, peak)) = high.take() {
                    found.push(event(
                        copy,
                        EventKind::MemoryHigh,
                        at,
                        None,
                        Some(i64::from(peak)),
                        Some(top),
                    ));
                }
            }
            previous = Some(metric.bucket);
            if let (Some(bytes), Some(share)) = (
                metric.disk,
                metric.disk.and_then(|b| percent(b, metric.disk_limit)),
            ) {
                for (index, threshold) in [DISK_ATTENTION, DISK_INCIDENT].into_iter().enumerate() {
                    if share >= threshold && armed[index] {
                        armed[index] = false;
                        found.push(event(
                            copy,
                            EventKind::DiskHigh,
                            metric.bucket,
                            None,
                            Some(i64::from(threshold)),
                            Some(bytes),
                        ));
                    }
                    if share + 5 < threshold {
                        armed[index] = true;
                    }
                }
            }
        }
        if let Some((at, top, peak)) = high {
            found.push(event(
                copy,
                EventKind::MemoryHigh,
                at,
                None,
                Some(i64::from(peak)),
                Some(top),
            ));
        }
    }

    let mut throttled: BTreeMap<i64, u32> = BTreeMap::new();
    for copy in input.copies {
        for metric in in_range(copy.metrics, from, to) {
            let held = throttled.entry(metric.bucket).or_default();
            *held = (*held).max(metric.throttled.unwrap_or(0));
        }
    }
    let mut run: Option<(i64, i64, u32)> = None;
    let close = |run: Option<(i64, i64, u32)>, found: &mut Vec<ServiceEvent>| {
        if let Some((start, end, seconds)) = run {
            let minutes = seconds / 60;
            if minutes >= THROTTLED_ATTENTION {
                found.push(ServiceEvent {
                    id: format!("throttled-{start}"),
                    service_id: String::new(),
                    container: None,
                    kind: EventKind::Throttled,
                    at: start,
                    until: Some(end),
                    code: Some(i64::from(minutes)),
                    detail: None,
                });
            }
        }
    };
    for (bucket, seconds) in throttled {
        run = match (run, seconds >= 60) {
            (Some((start, end, total)), true) if end == bucket => {
                Some((start, bucket + BUCKET_MS, total + seconds))
            }
            (other, true) => {
                close(other, &mut found);
                Some((bucket, bucket + BUCKET_MS, seconds))
            }
            (other, false) => {
                close(other, &mut found);
                None
            }
        };
    }
    close(run, &mut found);
    found.sort_by_key(|event| event.at);
    found
}

fn runs(
    buckets: impl Iterator<Item = i64>,
    make: impl Fn(i64, i64, usize) -> Option<ServiceEvent>,
    found: &mut Vec<ServiceEvent>,
) {
    let mut run: Option<(i64, i64, usize)> = None;
    for bucket in buckets {
        run = match run {
            Some((start, end, count)) if end == bucket => {
                Some((start, bucket + BUCKET_MS, count + 1))
            }
            other => {
                if let Some((start, end, count)) = other {
                    found.extend(make(start, end, count));
                }
                Some((bucket, bucket + BUCKET_MS, 1))
            }
        };
    }
    if let Some((start, end, count)) = run {
        found.extend(make(start, end, count));
    }
}

fn event(
    copy: &Copy,
    kind: EventKind,
    at: i64,
    until: Option<i64>,
    code: Option<i64>,
    bytes: Option<u64>,
) -> ServiceEvent {
    ServiceEvent {
        id: format!("{}-{}-{at}", kind.as_str(), copy.container),
        service_id: String::new(),
        container: Some(copy.container.to_string()),
        kind,
        at,
        until,
        code,
        detail: bytes.map(|bytes| bytes.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;
    const DAY0: i64 = 20_000 * DAY_MS;

    fn healthy(container: &str, from: i64, buckets: usize) -> Vec<CopyMetric> {
        (0..buckets)
            .map(|index| CopyMetric {
                container: container.into(),
                bucket: from + index as i64 * BUCKET_MS,
                samples: 10,
                running: 10,
                memory_max: Some(100 * MB),
                memory_limit: Some(256 * MB),
                cpu_avg: Some(50),
                cpu_max: Some(80),
                cpu_limit: Some(500),
                throttled: Some(0),
                received: Some(600_000),
                sent: Some(600_000),
                read: Some(0),
                written: Some(0),
                disk: None,
                disk_partial: false,
                disk_limit: None,
            })
            .collect()
    }

    fn stored(kind: EventKind, at: i64, container: Option<&str>) -> ServiceEvent {
        ServiceEvent {
            id: format!("{}-{at}", kind.as_str()),
            service_id: "svc".into(),
            container: container.map(str::to_string),
            kind,
            at,
            until: None,
            code: None,
            detail: None,
        }
    }

    fn judge(copies: &[Copy], events: &[ServiceEvent], rows: &[Row], now: i64) -> Vec<Day> {
        let input = Input {
            now,
            copies,
            events,
            rows,
            backup_every: Some(DAY_MS),
        };
        days(&input, DAY0, 3)
    }

    #[test]
    fn a_day_with_every_reading_and_nothing_happening_is_good() {
        let metrics = healthy("p.api", DAY0, 432);
        let copies = [Copy {
            slot: 1,
            container: "p.api",
            metrics: &metrics,
            current: true,
        }];
        let days = judge(
            &copies,
            &[],
            &[Row::Availability, Row::Memory, Row::Cpu],
            DAY0 + 3 * DAY_MS,
        );
        for day in &days {
            for verdict in day.verdicts.values() {
                assert_eq!(verdict.state, State::Good, "{verdict:?}");
            }
        }
    }

    #[test]
    fn a_day_before_anything_was_measured_has_no_data_rather_than_a_gap() {
        let metrics = healthy("p.api", DAY0 + 2 * DAY_MS, 100);
        let copies = [Copy {
            slot: 1,
            container: "p.api",
            metrics: &metrics,
            current: true,
        }];
        let days = judge(
            &copies,
            &[],
            &[Row::Availability],
            DAY0 + 2 * DAY_MS + 100 * BUCKET_MS,
        );
        assert_eq!(days[0].verdicts[&Row::Availability].state, State::NoData);
        assert_eq!(days[2].verdicts[&Row::Availability].state, State::Good);
    }

    #[test]
    fn half_an_hour_unmeasured_is_an_incident_naming_the_copy() {
        let mut node_b = healthy("p.api.2", DAY0, 432);
        node_b.retain(|m| !(m.bucket >= DAY0 + 12 * BUCKET_MS && m.bucket < DAY0 + 15 * BUCKET_MS));
        let node_a = healthy("p.api", DAY0, 432);
        let copies = [
            Copy {
                slot: 1,
                container: "p.api",
                metrics: &node_a,
                current: true,
            },
            Copy {
                slot: 2,
                container: "p.api.2",
                metrics: &node_b,
                current: true,
            },
        ];
        let input = Input {
            now: DAY0 + 3 * DAY_MS,
            copies: &copies,
            events: &[],
            rows: &[Row::Availability],
            backup_every: None,
        };
        let days = days(&input, DAY0, 1);
        assert_eq!(
            days[0].verdicts[&Row::Availability],
            Verdict::new(
                State::Incident,
                Reason::Unmeasured {
                    slot: Some(2),
                    minutes: 30
                }
            )
        );
        let derived = derived(&input, DAY0, DAY0 + DAY_MS);
        assert_eq!(derived.len(), 1);
        assert_eq!(derived[0].kind, EventKind::Unmeasured);
        assert_eq!(derived[0].code, Some(30));
    }

    #[test]
    fn a_copy_that_was_meant_to_be_stopped_is_not_missing() {
        let mut metrics = healthy("p.api", DAY0, 432);
        metrics.retain(|m| !(m.bucket >= DAY0 + 6 * BUCKET_MS && m.bucket < DAY0 + 60 * BUCKET_MS));
        let events = [
            stored(EventKind::Stopped, DAY0 + 6 * BUCKET_MS, None),
            stored(EventKind::Started, DAY0 + 60 * BUCKET_MS, None),
        ];
        let copies = [Copy {
            slot: 1,
            container: "p.api",
            metrics: &metrics,
            current: true,
        }];
        let days = judge(&copies, &events, &[Row::Availability], DAY0 + 3 * DAY_MS);
        assert_eq!(days[0].verdicts[&Row::Availability].state, State::Good);
    }

    #[test]
    fn an_oom_kill_is_an_incident_for_memory_and_attention_for_availability() {
        let metrics = healthy("p.api", DAY0, 432);
        let events = [stored(
            EventKind::OomKilled,
            DAY0 + 3_600_000,
            Some("p.api"),
        )];
        let copies = [Copy {
            slot: 1,
            container: "p.api",
            metrics: &metrics,
            current: true,
        }];
        let days = judge(
            &copies,
            &events,
            &[Row::Availability, Row::Memory],
            DAY0 + 3 * DAY_MS,
        );
        assert_eq!(days[0].verdicts[&Row::Memory].state, State::Incident);
        assert_eq!(days[0].verdicts[&Row::Availability].state, State::Attention);
        assert_eq!(days[0].stats.oom_kills, 1);
    }

    #[test]
    fn memory_past_nine_tenths_of_its_ceiling_asks_for_attention() {
        let mut metrics = healthy("p.api", DAY0, 432);
        metrics[30].memory_max = Some(240 * MB);
        let copies = [Copy {
            slot: 1,
            container: "p.api",
            metrics: &metrics,
            current: true,
        }];
        let input = Input {
            now: DAY0 + 3 * DAY_MS,
            copies: &copies,
            events: &[],
            rows: &[Row::Memory],
            backup_every: None,
        };
        let days = days(&input, DAY0, 1);
        assert_eq!(
            days[0].verdicts[&Row::Memory],
            Verdict::new(
                State::Attention,
                Reason::MemoryPeak {
                    bytes: 240 * MB,
                    percent: Some(93)
                }
            )
        );
        let derived = derived(&input, DAY0, DAY0 + DAY_MS);
        assert_eq!(derived[0].kind, EventKind::MemoryHigh);
        assert_eq!(derived[0].code, Some(93));
    }

    #[test]
    fn eighty_minutes_at_the_cpu_ceiling_is_attention_and_one_event() {
        let mut metrics = healthy("p.api", DAY0, 432);
        for metric in &mut metrics[80..88] {
            metric.throttled = Some(600);
        }
        let copies = [Copy {
            slot: 1,
            container: "p.api",
            metrics: &metrics,
            current: true,
        }];
        let input = Input {
            now: DAY0 + 3 * DAY_MS,
            copies: &copies,
            events: &[],
            rows: &[Row::Cpu],
            backup_every: None,
        };
        let days = days(&input, DAY0, 1);
        assert_eq!(
            days[0].verdicts[&Row::Cpu],
            Verdict::new(State::Attention, Reason::Throttled { minutes: 80 })
        );
        let derived = derived(&input, DAY0, DAY0 + DAY_MS);
        assert_eq!(derived.len(), 1);
        assert_eq!(
            (derived[0].kind, derived[0].code),
            (EventKind::Throttled, Some(80))
        );
    }

    #[test]
    fn a_disk_growing_toward_its_ceiling_says_when_it_fills() {
        let mut metrics = healthy("p.cms", DAY0 - 7 * DAY_MS, 144 * 10);
        for (index, metric) in metrics.iter_mut().enumerate() {
            metric.disk = Some(3_000 * MB + index as u64 * MB / 3);
            metric.disk_limit = Some(4_000 * MB);
        }
        let copies = [Copy {
            slot: 1,
            container: "p.cms",
            metrics: &metrics,
            current: true,
        }];
        let days = judge(&copies, &[], &[Row::Disk], DAY0 + 3 * DAY_MS);
        let Reason::Disk {
            percent,
            full_in_days,
            ..
        } = &days[0].verdicts[&Row::Disk].reason
        else {
            panic!("{:?}", days[0].verdicts[&Row::Disk]);
        };
        assert_eq!(*percent, Some(84));
        assert!(
            full_in_days.is_some_and(|days| days < 14),
            "{full_in_days:?}"
        );
        assert_eq!(days[0].verdicts[&Row::Disk].state, State::Attention);
    }

    #[test]
    fn a_failed_backup_with_no_good_one_since_is_an_incident_and_a_retry_is_attention() {
        let metrics = healthy("p.db", DAY0 - 2 * DAY_MS, 144 * 5);
        let copies = [Copy {
            slot: 1,
            container: "p.db",
            metrics: &metrics,
            current: true,
        }];
        let events = [
            stored(EventKind::BackedUp, DAY0 - DAY_MS + 3 * 3_600_000, None),
            stored(EventKind::BackupFailed, DAY0 + 3 * 3_600_000, None),
            stored(EventKind::BackupFailed, DAY0 + DAY_MS + 3 * 3_600_000, None),
            stored(EventKind::BackedUp, DAY0 + DAY_MS + 4 * 3_600_000, None),
        ];
        let days = judge(&copies, &events, &[Row::Backups], DAY0 + 3 * DAY_MS);
        assert_eq!(days[0].verdicts[&Row::Backups].state, State::Incident);
        assert_eq!(days[1].verdicts[&Row::Backups].state, State::Attention);
        assert_eq!(days[2].verdicts[&Row::Backups].state, State::Incident);
    }

    #[test]
    fn a_standby_not_following_for_half_an_hour_is_an_incident() {
        let metrics = healthy("p.db", DAY0, 432);
        let copies = [Copy {
            slot: 1,
            container: "p.db",
            metrics: &metrics,
            current: true,
        }];
        let mut unfollowed = stored(EventKind::Unfollowed, DAY0 + 6 * 3_600_000, Some("p.db.2"));
        unfollowed.until = Some(DAY0 + 6 * 3_600_000 + 30 * MINUTE_MS);
        let days = judge(&copies, &[unfollowed], &[Row::Replicas], DAY0 + 3 * DAY_MS);
        assert_eq!(
            days[0].verdicts[&Row::Replicas],
            Verdict::new(State::Incident, Reason::Unfollowed { minutes: 30 })
        );
        assert_eq!(days[1].verdicts[&Row::Replicas].state, State::Good);
    }

    #[test]
    fn the_page_opens_on_the_latest_day_that_was_not_good() {
        let metrics = healthy("p.api", DAY0, 432);
        let events = [stored(
            EventKind::OomKilled,
            DAY0 + DAY_MS + 3_600_000,
            Some("p.api"),
        )];
        let copies = [Copy {
            slot: 1,
            container: "p.api",
            metrics: &metrics,
            current: true,
        }];
        let days = judge(&copies, &events, &[Row::Memory], DAY0 + 3 * DAY_MS);
        assert_eq!(worth_opening(&days), Some(DAY0 + DAY_MS));
        let quiet = judge(&copies, &[], &[Row::Memory], DAY0 + 3 * DAY_MS);
        assert_eq!(worth_opening(&quiet), None);
    }

    #[test]
    fn twice_the_usual_traffic_asks_for_attention() {
        let mut metrics = healthy("p.api", DAY0 - 7 * DAY_MS, 144 * 10);
        let spike = (7 * 144 + 50) as usize;
        metrics[spike].received = Some(3_000_000);
        let copies = [Copy {
            slot: 1,
            container: "p.api",
            metrics: &metrics,
            current: true,
        }];
        let days = judge(&copies, &[], &[Row::Network], DAY0 + 3 * DAY_MS);
        assert_eq!(days[0].verdicts[&Row::Network].state, State::Attention);
        assert_eq!(days[1].verdicts[&Row::Network].state, State::Good);
    }
}
