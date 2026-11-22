//! What every copy on this node did, a reading a minute folded into ten
//! minutes (`docs/stability.md`).
//!
//! The kernel's counters are totals since the container started, so a
//! reading means something only against the one before it, and a counter
//! that went backwards is a container that was recreated: that interval is
//! dropped rather than guessed.

pub mod reading;

use std::collections::BTreeMap;

use crate::domain::entities::copy_metric::{bucket_of, CopyMetric};

pub use reading::Reading;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub container: String,
    pub reading: Option<Reading>,
}

#[derive(Debug, Clone, Default)]
struct Totals {
    samples: u32,
    running: u32,
    memory_max: Option<u64>,
    memory_limit: Option<u64>,
    cpu_limit: Option<u32>,
    cpu_usec: u64,
    cpu_elapsed_ms: u64,
    cpu_max: Option<u32>,
    throttled_usec: u64,
    measured_throttle: bool,
    received: Option<u64>,
    sent: Option<u64>,
    read: Option<u64>,
    written: Option<u64>,
}

#[derive(Debug, Clone)]
struct Previous {
    at: i64,
    reading: Reading,
}

#[derive(Debug, Default)]
pub struct Sampler {
    bucket: Option<i64>,
    totals: BTreeMap<String, Totals>,
    previous: BTreeMap<String, Previous>,
}

fn add(total: &mut Option<u64>, delta: Option<u64>) {
    if let Some(delta) = delta {
        *total = Some(total.unwrap_or(0) + delta);
    }
}

fn grew(now: u64, before: u64) -> Option<u64> {
    now.checked_sub(before)
}

impl Sampler {
    /// Take one minute's readings, and hand back the ten minutes they closed,
    /// if they closed any.
    pub fn observe(&mut self, now: i64, observations: Vec<Observation>) -> Vec<CopyMetric> {
        let bucket = bucket_of(now);
        let closed = match self.bucket {
            Some(open) if open != bucket => self.close(open),
            _ => Vec::new(),
        };
        self.bucket = Some(bucket);

        let present: Vec<String> = observations.iter().map(|o| o.container.clone()).collect();
        self.previous
            .retain(|container, _| present.contains(container));
        for observation in observations {
            let totals = self
                .totals
                .entry(observation.container.clone())
                .or_default();
            totals.samples += 1;
            let Some(reading) = observation.reading else {
                self.previous.remove(&observation.container);
                continue;
            };
            totals.running += 1;
            totals.memory_max = Some(totals.memory_max.unwrap_or(0).max(reading.memory));
            totals.memory_limit = reading.memory_limit.or(totals.memory_limit);
            totals.cpu_limit = reading.cpu_limit.or(totals.cpu_limit);
            if let Some(before) = self.previous.get(&observation.container) {
                let elapsed = (now - before.at).max(0) as u64;
                let same = before.reading.pid == reading.pid;
                if let (true, true, Some(used)) = (
                    same,
                    elapsed > 0,
                    grew(reading.cpu_usec, before.reading.cpu_usec),
                ) {
                    totals.cpu_usec += used;
                    totals.cpu_elapsed_ms += elapsed;
                    let rate = (used / elapsed) as u32;
                    totals.cpu_max = Some(totals.cpu_max.unwrap_or(0).max(rate));
                    if let Some(periods) =
                        grew(reading.throttled_periods, before.reading.throttled_periods)
                    {
                        totals.throttled_usec += periods * reading.period_usec;
                        totals.measured_throttle = true;
                    }
                }
                if same {
                    add(
                        &mut totals.received,
                        delta(reading.received, before.reading.received),
                    );
                    add(&mut totals.sent, delta(reading.sent, before.reading.sent));
                    add(&mut totals.read, delta(reading.read, before.reading.read));
                    add(
                        &mut totals.written,
                        delta(reading.written, before.reading.written),
                    );
                }
            }
            self.previous
                .insert(observation.container, Previous { at: now, reading });
        }
        closed
    }

    fn close(&mut self, bucket: i64) -> Vec<CopyMetric> {
        std::mem::take(&mut self.totals)
            .into_iter()
            .map(|(container, totals)| CopyMetric {
                container,
                bucket,
                samples: totals.samples,
                running: totals.running,
                memory_max: totals.memory_max,
                memory_limit: totals.memory_limit,
                cpu_avg: (totals.cpu_elapsed_ms > 0)
                    .then(|| (totals.cpu_usec / totals.cpu_elapsed_ms) as u32),
                cpu_max: totals.cpu_max,
                cpu_limit: totals.cpu_limit,
                throttled: totals
                    .measured_throttle
                    .then_some((totals.throttled_usec / 1_000_000) as u32),
                received: totals.received,
                sent: totals.sent,
                read: totals.read,
                written: totals.written,
                disk: None,
                disk_partial: false,
                disk_limit: None,
            })
            .collect()
    }
}

fn delta(now: Option<u64>, before: Option<u64>) -> Option<u64> {
    grew(now?, before?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: i64 = 60_000;

    fn reading(pid: u32, cpu_seconds: u64, throttled: u64, received: u64) -> Reading {
        Reading {
            pid,
            memory: 100 + cpu_seconds,
            memory_limit: Some(256),
            cpu_limit: Some(500),
            cpu_usec: cpu_seconds * 1_000_000,
            throttled_periods: throttled,
            period_usec: 100_000,
            received: Some(received),
            sent: Some(received * 2),
            read: Some(0),
            written: Some(0),
        }
    }

    fn up(container: &str, reading: Reading) -> Observation {
        Observation {
            container: container.into(),
            reading: Some(reading),
        }
    }

    #[test]
    fn ten_minutes_close_on_the_first_reading_after_them() {
        let mut sampler = Sampler::default();
        for minute in 0..10 {
            let closed = sampler.observe(
                minute * MINUTE,
                vec![up(
                    "p.api",
                    reading(7, 30 * minute as u64, 0, 1000 * minute as u64),
                )],
            );
            assert!(closed.is_empty());
        }
        let closed = sampler.observe(10 * MINUTE, vec![up("p.api", reading(7, 300, 0, 10_000))]);
        assert_eq!(closed.len(), 1);
        let row = &closed[0];
        assert_eq!((row.bucket, row.samples, row.running), (0, 10, 10));
        assert_eq!(
            row.cpu_avg,
            Some(500),
            "thirty seconds a minute is half a core"
        );
        assert_eq!(row.received, Some(9_000));
        assert_eq!(row.sent, Some(18_000));
        assert_eq!(row.memory_max, Some(370));
        assert_eq!(row.throttled, Some(0));
    }

    #[test]
    fn a_copy_that_was_not_running_counts_a_sample_and_nothing_else() {
        let mut sampler = Sampler::default();
        sampler.observe(
            0,
            vec![Observation {
                container: "p.api".into(),
                reading: None,
            }],
        );
        sampler.observe(MINUTE, vec![up("p.api", reading(7, 1, 0, 0))]);
        let closed = sampler.observe(10 * MINUTE, vec![]);
        assert_eq!((closed[0].samples, closed[0].running), (2, 1));
        assert_eq!(closed[0].cpu_avg, None, "one reading is not a rate");
    }

    #[test]
    fn a_recreated_container_drops_the_interval_rather_than_counting_backwards() {
        let mut sampler = Sampler::default();
        sampler.observe(0, vec![up("p.api", reading(7, 500, 40, 1_000_000))]);
        sampler.observe(MINUTE, vec![up("p.api", reading(9, 2, 0, 10))]);
        sampler.observe(2 * MINUTE, vec![up("p.api", reading(9, 32, 600, 1_010))]);
        let closed = sampler.observe(10 * MINUTE, vec![]);
        assert_eq!(closed[0].cpu_avg, Some(500));
        assert_eq!(closed[0].received, Some(1_000));
        assert_eq!(
            closed[0].throttled,
            Some(60),
            "600 periods of 100 ms held at the ceiling"
        );
    }
}
