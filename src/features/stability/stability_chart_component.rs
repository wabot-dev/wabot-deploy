//! A day's charts, drawn here so they are on the page without a script:
//! the SVG is numbers and class names only, never text from a row.

use std::fmt::Write;

use crate::domain::entities::copy_metric::{CopyMetric, BUCKET_MS};
use crate::domain::policies::stability::{Reason, Row, DAY_MS};
use crate::features::stability::service_stability_response::{Chart, ServiceStabilityResponse};
use crate::features::stability::texts::{self, t};

const FULL: usize = (DAY_MS / BUCKET_MS) as usize;
const SECONDS: f64 = (BUCKET_MS / 1000) as f64;
const MIB: f64 = 1024.0 * 1024.0;
const GIB: f64 = 1024.0 * MIB;

pub struct Plot {
    pub title: &'static str,
    pub summary: String,
    pub ticks: Vec<(f64, String)>,
    pub svg: String,
    pub mirrored: Option<(&'static str, &'static str)>,
    pub data: serde_json::Value,
}

fn x(index: usize) -> f64 {
    (index as f64 + 0.5) / FULL as f64 * 1000.0
}

fn nice(value: f64) -> f64 {
    if value <= 0.0 || !value.is_finite() {
        return 1.0;
    }
    let base = 10f64.powf(value.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .find(|step| value / base <= *step)
        .unwrap_or(10.0);
    step * base
}

fn nice_bytes(value: f64) -> f64 {
    match value {
        v if v >= GIB => nice(v / GIB) * GIB,
        v if v >= MIB => nice(v / MIB) * MIB,
        v => nice(v),
    }
}

fn line(values: &[Option<f64>], y: impl Fn(f64) -> f64) -> String {
    let mut path = String::new();
    let mut run = 0;
    for (index, value) in values.iter().enumerate() {
        match value {
            Some(value) => {
                let pen = if run == 0 { 'M' } else { 'L' };
                let _ = write!(path, "{pen}{:.1} {:.2}", x(index), y(*value));
                run += 1;
            }
            None => {
                if run == 1 {
                    path.push_str("h0");
                }
                run = 0;
            }
        }
    }
    if run == 1 {
        path.push_str("h0");
    }
    path
}

fn area(values: &[Option<f64>], y: impl Fn(f64) -> f64) -> String {
    let mut path = String::new();
    let mut open: Option<usize> = None;
    let mut last = 0;
    for (index, value) in values.iter().enumerate() {
        match (value, open) {
            (Some(value), None) => {
                let _ = write!(path, "M{:.1} 50L{:.1} {:.2}", x(index), x(index), y(*value));
                open = Some(index);
                last = index;
            }
            (Some(value), Some(_)) => {
                let _ = write!(path, "L{:.1} {:.2}", x(index), y(*value));
                last = index;
            }
            (None, Some(_)) => {
                let _ = write!(path, "L{:.1} 50Z", x(last));
                open = None;
            }
            (None, None) => {}
        }
    }
    if open.is_some() {
        let _ = write!(path, "L{:.1} 50Z", x(last));
    }
    path
}

fn steps(limits: &[Option<f64>], y: impl Fn(f64) -> f64) -> String {
    let mut path = String::new();
    let mut previous: Option<f64> = None;
    for (index, limit) in limits.iter().enumerate() {
        let edge = index as f64 / FULL as f64 * 1000.0;
        match (previous, limit) {
            (None, Some(limit)) => {
                let _ = write!(path, "M{edge:.1} {:.2}", y(*limit));
            }
            (Some(before), Some(limit)) if before != *limit => {
                let _ = write!(path, "H{edge:.1}V{:.2}", y(*limit));
            }
            (Some(_), None) => {
                let _ = write!(path, "H{edge:.1}");
            }
            _ => {}
        }
        previous = *limit;
    }
    if previous.is_some() {
        path.push_str("H1000");
    }
    path
}

fn colour(index: usize) -> &'static str {
    ["c1", "c2", "c3", "c4"].get(index).copied().unwrap_or("c0")
}

struct Lines<'a> {
    title: &'static str,
    unit: &'static str,
    series: Vec<Vec<Option<f64>>>,
    limits: Vec<Option<f64>>,
    summary: String,
    scale: &'a dyn Fn(f64) -> f64,
    label: &'a dyn Fn(f64) -> String,
}

fn lines(chart: Lines) -> Plot {
    let peak = chart
        .series
        .iter()
        .flatten()
        .flatten()
        .copied()
        .fold(0.0, f64::max);
    let ceiling = chart.limits.iter().flatten().copied().fold(0.0, f64::max);
    let top = (chart.scale)(peak.max(ceiling) * 1.15);
    let y = |value: f64| 100.0 - value.min(top) / top * 100.0;
    let mut svg = String::from(
        r#"<svg viewBox="0 0 1000 100" preserveAspectRatio="none" aria-hidden="true">"#,
    );
    let marks: Vec<f64> = match ceiling > 0.0 {
        true => vec![ceiling, ceiling / 2.0],
        false => vec![top, top / 2.0],
    };
    let _ = write!(
        svg,
        r#"<path class="st-grid" d="M0 {:.2}H1000"/>"#,
        y(marks[1])
    );
    svg.push_str(r#"<path class="st-axis" d="M0 100H1000"/>"#);
    for (index, values) in chart.series.iter().enumerate().rev() {
        let _ = write!(
            svg,
            r#"<path class="st-ln {}" d="{}"/>"#,
            colour(index),
            line(values, y)
        );
    }
    let ceil = steps(&chart.limits, y);
    if !ceil.is_empty() {
        let _ = write!(svg, r#"<path class="st-ceil" d="{ceil}"/>"#);
    }
    svg.push_str("</svg>");
    let mut ticks: Vec<(f64, String)> = marks
        .iter()
        .map(|mark| (y(*mark), (chart.label)(*mark)))
        .collect();
    ticks.push((100.0, "0".to_string()));
    let measured = chart.series.iter().flatten().any(Option::is_some);
    Plot {
        title: chart.title,
        summary: match measured {
            true => chart.summary,
            false => t("no data").to_string(),
        },
        ticks: if measured { ticks } else { Vec::new() },
        svg,
        mirrored: None,
        data: serde_json::json!({
            "kind": "lines",
            "unit": chart.unit,
            "top": top,
            "series": chart.series,
            "ceil": chart.limits,
        }),
    }
}

struct Mirrored {
    title: &'static str,
    up: Vec<Option<f64>>,
    down: Vec<Option<f64>>,
    labels: (&'static str, &'static str),
    summary: String,
}

fn mirrored(chart: Mirrored) -> Plot {
    let peak = chart
        .up
        .iter()
        .chain(chart.down.iter())
        .flatten()
        .copied()
        .fold(0.0, f64::max);
    let top = nice_bytes((peak * 1.08).max(100.0));
    let up = |value: f64| 50.0 - value.min(top) / top * 46.0;
    let down = |value: f64| 50.0 + value.min(top) / top * 46.0;
    let mut svg = String::from(
        r#"<svg viewBox="0 0 1000 100" preserveAspectRatio="none" aria-hidden="true">"#,
    );
    svg.push_str(
        r#"<path class="st-grid" d="M0 4H1000M0 96H1000"/><path class="st-axis" d="M0 50H1000"/>"#,
    );
    let _ = write!(
        svg,
        r#"<path class="st-area" d="{}{}"/><path class="st-ln st-flow" d="{}"/><path class="st-ln st-flow" d="{}"/></svg>"#,
        area(&chart.up, up),
        area(&chart.down, down),
        line(&chart.up, up),
        line(&chart.down, down),
    );
    let label = texts::rate(top as u64);
    let measured = chart
        .up
        .iter()
        .chain(chart.down.iter())
        .any(Option::is_some);
    Plot {
        title: chart.title,
        summary: match measured {
            true => chart.summary,
            false => t("no data").to_string(),
        },
        ticks: match measured {
            true => vec![(4.0, label.clone()), (50.0, "0".to_string()), (96.0, label)],
            false => Vec::new(),
        },
        svg,
        mirrored: Some(chart.labels),
        data: serde_json::json!({
            "kind": "mirrored",
            "unit": "rate",
            "top": top,
            "up": chart.up,
            "down": chart.down,
            "labels": [chart.labels.0, chart.labels.1],
        }),
    }
}

fn each(
    series: &[Vec<Option<CopyMetric>>],
    value: impl Fn(&CopyMetric) -> Option<f64>,
) -> Vec<Vec<Option<f64>>> {
    series
        .iter()
        .map(|line| {
            line.iter()
                .map(|metric| metric.as_ref().and_then(&value))
                .collect()
        })
        .collect()
}

fn summed(
    series: &[Vec<Option<CopyMetric>>],
    value: impl Fn(&CopyMetric) -> Option<u64>,
) -> Vec<Option<f64>> {
    let length = series.first().map(Vec::len).unwrap_or(0);
    (0..length)
        .map(|index| {
            let mut total = None;
            for line in series {
                if let Some(bytes) = line[index]
                    .as_ref()
                    .filter(|m| m.running > 0)
                    .and_then(&value)
                {
                    total = Some(total.unwrap_or(0.0) + bytes as f64 / SECONDS);
                }
            }
            total
        })
        .collect()
}

fn limits(
    series: &[Vec<Option<CopyMetric>>],
    value: impl Fn(&CopyMetric) -> Option<u64>,
) -> Vec<Option<f64>> {
    let length = series.first().map(Vec::len).unwrap_or(0);
    (0..length)
        .map(|index| {
            series
                .iter()
                .filter_map(|line| line[index].as_ref().and_then(&value))
                .max()
                .map(|limit| limit as f64)
        })
        .collect()
}

fn peak_of(values: &[Vec<Option<f64>>]) -> f64 {
    values
        .iter()
        .flatten()
        .flatten()
        .copied()
        .fold(0.0, f64::max)
}

pub fn plots(response: &ServiceStabilityResponse) -> Vec<Plot> {
    let series = &response.series;
    let today = response
        .strip
        .iter()
        .find(|day| day.start == response.selected);
    response
        .charts
        .iter()
        .map(|chart| match chart {
            Chart::Memory => {
                let values = each(series, |m| m.memory_max.map(|v| v as f64));
                let ceilings = limits(series, |m| m.memory_limit);
                let peak = peak_of(&values);
                let ceiling = ceilings.iter().flatten().copied().fold(0.0, f64::max);
                let summary = match ceiling > 0.0 {
                    true => format!(
                        "{} · {} %",
                        texts::peak(texts::bytes(peak as u64)),
                        (peak / ceiling * 100.0).floor()
                    ),
                    false => texts::peak(texts::bytes(peak as u64)),
                };
                lines(Lines {
                    title: t("Memory"),
                    unit: "bytes",
                    series: values,
                    limits: ceilings,
                    summary,
                    scale: &nice_bytes,
                    label: &|value| texts::bytes(value as u64),
                })
            }
            Chart::Cpu => {
                let values = each(series, |m| m.cpu_max.map(f64::from));
                let ceilings = limits(series, |m| m.cpu_limit.map(u64::from));
                let mut summary = texts::peak(texts::millicores(peak_of(&values) as u32));
                let throttled = today.map(|day| day.stats.throttled_minutes).unwrap_or(0);
                if throttled > 0 {
                    summary.push_str(&format!(" · {}", texts::minutes_at_ceiling(throttled)));
                }
                lines(Lines {
                    title: t("CPU"),
                    unit: "millicores",
                    series: values,
                    limits: ceilings,
                    summary,
                    scale: &nice,
                    label: &|value| texts::millicores(value as u32),
                })
            }
            Chart::Network => {
                let up = summed(series, |m| m.received);
                let down = summed(series, |m| m.sent);
                let summary = format!(
                    "{} {} · {} {}",
                    t("In").to_lowercase(),
                    texts::rate(peak_of(std::slice::from_ref(&up)) as u64),
                    t("Out").to_lowercase(),
                    texts::rate(peak_of(std::slice::from_ref(&down)) as u64),
                );
                mirrored(Mirrored {
                    title: t("Network"),
                    up,
                    down,
                    labels: (t("In"), t("Out")),
                    summary: texts::peak(summary),
                })
            }
            Chart::Disk => {
                let values = each(series, |m| m.disk.map(|v| v as f64));
                let ceilings = limits(series, |m| m.disk_limit);
                let summary = match today.and_then(|day| day.verdicts.get(&Row::Disk)) {
                    Some(verdict) if matches!(verdict.reason, Reason::Disk { .. }) => {
                        texts::why(&verdict.reason)
                    }
                    _ => texts::bytes(peak_of(&values) as u64),
                };
                lines(Lines {
                    title: t("Disk"),
                    unit: "bytes",
                    series: values,
                    limits: ceilings,
                    summary,
                    scale: &nice_bytes,
                    label: &|value| texts::bytes(value as u64),
                })
            }
            Chart::Io => {
                let up = summed(series, |m| m.read);
                let down = summed(series, |m| m.written);
                let summary = format!(
                    "{} {} · {} {}",
                    t("Read").to_lowercase(),
                    texts::rate(peak_of(std::slice::from_ref(&up)) as u64),
                    t("Write").to_lowercase(),
                    texts::rate(peak_of(std::slice::from_ref(&down)) as u64),
                );
                mirrored(Mirrored {
                    title: t("Disk I/O"),
                    up,
                    down,
                    labels: (t("Read"), t("Write")),
                    summary: texts::peak(summary),
                })
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_in_the_readings_breaks_the_line_rather_than_bridging_it() {
        let drawn = line(&[Some(1.0), Some(2.0), None, Some(3.0), None], |v| v);
        assert_eq!(drawn.matches('M').count(), 2);
        assert!(
            drawn.ends_with("h0"),
            "a lone reading is still a dot: {drawn}"
        );
    }

    #[test]
    fn a_ceiling_raised_halfway_is_drawn_as_a_step() {
        let mut limits = vec![Some(200.0); FULL / 2];
        limits.extend(vec![Some(300.0); FULL / 2]);
        assert_eq!(steps(&limits, |v| v), "M0.0 200.00H500.0V300.00H1000");
    }

    #[test]
    fn an_axis_ends_on_a_round_number() {
        assert_eq!(nice(437.0), 500.0);
        assert_eq!(nice_bytes(300.0 * MIB), 500.0 * MIB);
    }
}
