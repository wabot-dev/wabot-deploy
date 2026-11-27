//! The day-by-day card: a row of days for each thing judged, every cell a
//! link to that day.

use hypertext::prelude::*;

use crate::domain::policies::stability::State;
use crate::features::stability::see_service_stability_request::day_param;
use crate::features::stability::service_stability_response::ServiceStabilityResponse;
use crate::features::stability::texts::{self, t};
use crate::shell::components::declaration;

struct Cell {
    class: String,
    href: Option<String>,
    tip: String,
    head: String,
    day: String,
    why: String,
    state: State,
}

fn class(state: State, selected: bool) -> String {
    let state = match state {
        State::Good => "ok",
        State::Attention => "warn",
        State::Incident => "crit",
        State::NoData => "none",
    };
    match selected {
        true => format!("st-cell {state} sel"),
        false => format!("st-cell {state}"),
    }
}

fn cells(response: &ServiceStabilityResponse, base: &str) -> Vec<(&'static str, Vec<Cell>)> {
    response
        .rows
        .iter()
        .map(|row| {
            let cells = response
                .strip
                .iter()
                .filter_map(|day| {
                    let verdict = day.verdicts.get(row)?;
                    let head = format!("{}: {}", texts::row(*row), texts::state(verdict.state));
                    let why = texts::why(&verdict.reason);
                    let date = texts::full_day(day.start);
                    let reachable = response.since.is_some_and(|since| day.start >= since);
                    Some(Cell {
                        class: class(verdict.state, day.start == response.selected),
                        href: reachable.then(|| {
                            format!(
                                "{base}?days={}&day={}#day",
                                response.days,
                                day_param(&response.zone, day.start)
                            )
                        }),
                        tip: format!("{date} · {head} · {why}"),
                        head,
                        day: date,
                        why,
                        state: verdict.state,
                    })
                })
                .collect();
            (texts::row(*row), cells)
        })
        .collect()
}

pub fn strip<'a>(response: &'a ServiceStabilityResponse, base: &'a str) -> impl Renderable + 'a {
    let count = response.strip.len().max(1);
    let gap = if count > 45 { 2 } else { 3 };
    let every = if count > 45 { 15 } else { 7 };
    let labels: Vec<(f64, String)> = response
        .strip
        .iter()
        .enumerate()
        .filter(|(index, _)| (count - 1 - index).is_multiple_of(every))
        .map(|(index, day)| {
            let left = (index as f64 + 0.5) / count as f64 * 100.0;
            let label = match day.start == response.today {
                true => t("today").to_string(),
                false => texts::short_day(day.start),
            };
            (left, label)
        })
        .collect();
    let deploys: Vec<Option<&str>> = response
        .strip
        .iter()
        .map(|day| {
            response
                .deploys
                .iter()
                .find(|(start, _)| *start == day.start)
                .map(|(_, tag)| tag.as_str())
        })
        .collect();
    let has_none = response.strip.iter().any(|day| {
        day.verdicts
            .values()
            .any(|verdict| verdict.state == State::NoData)
    });
    let rows = cells(response, base);
    rsx! {
        <section class="card st-card">
            <div class="split">
                <h2>(t("Day by day"))</h2>
                <div class="st-legend">
                    <span><i class="st-key ok"></i>(texts::state(State::Good))</span>
                    <span><i class="st-key warn"><i class="st-g-bar"></i></i>(texts::state(State::Attention))</span>
                    <span><i class="st-key crit"><i class="st-g-dot"></i></i>(texts::state(State::Incident))</span>
                    @if has_none {
                        <span><i class="st-key none"></i>(texts::state(State::NoData))</span>
                    }
                </div>
            </div>
            <div class="st-strips" style=(declaration::cells(count, gap))>
                <div class="st-row">
                    <span class="st-label st-quiet">(t("Deploys"))</span>
                    <div class="st-deps">
                        @for deploy in &deploys {
                            <span class="st-dep">
                                @if let Some(tag) = deploy {
                                    <span>(tag)</span><i></i>
                                }
                            </span>
                        }
                    </div>
                </div>
                @for (label, row) in &rows {
                    <div class="st-row">
                        <span class="st-label">(label)</span>
                        <div class="st-cells">
                            @for cell in row {
                                @if let Some(href) = &cell.href {
                                    <a class=(&cell.class) href=(href) title=(&cell.tip) aria-label=(&cell.tip)
                                       data-st-head=(&cell.head) data-st-day=(&cell.day) data-st-why=(&cell.why)>
                                        @if cell.state == State::Attention { <i class="st-g-bar"></i> }
                                        @if cell.state == State::Incident { <i class="st-g-dot"></i> }
                                    </a>
                                } @else {
                                    <span class=(&cell.class) title=(&cell.tip) aria-label=(&cell.tip)></span>
                                }
                            }
                        </div>
                    </div>
                }
                <div class="st-row">
                    <span></span>
                    <div class="st-xl">
                        @for (left, label) in &labels {
                            <span style=(declaration::left(*left))>(label)</span>
                        }
                    </div>
                </div>
            </div>
        </section>
    }
}
