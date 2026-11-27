//! Everything that happened in the period, newest first, each a link to
//! its day with it pinned.

use hypertext::prelude::*;

use crate::features::stability::see_service_stability_request::day_param;
use crate::features::stability::service_stability_response::ServiceStabilityResponse;
use crate::features::stability::stability_day_component::{severity, who};
use crate::features::stability::texts::{self, t};

struct Line {
    class: &'static str,
    dot: String,
    when: String,
    href: String,
    title: String,
    who: String,
    detail: Option<(String, bool)>,
}

pub fn events<'a>(response: &'a ServiceStabilityResponse, base: &'a str) -> impl Renderable + 'a {
    let copies = response.copies.len();
    let when = match response.zone.reader_unknown() {
        true => format!("{} · UTC", t("When")),
        false => t("When").to_string(),
    };
    let lines: Vec<Line> = response
        .period_events
        .iter()
        .map(|event| {
            let mut when = texts::moment(event.at);
            if let Some(until) = event.until {
                when.push_str(&format!("–{}", texts::clock(until)));
            }
            Line {
                class: if response.pinned.as_deref() == Some(event.id.as_str()) {
                    "on"
                } else {
                    ""
                },
                dot: format!("st-dot {}", severity(event.severity)),
                when,
                href: format!(
                    "{base}?days={}&day={}&event={}#day",
                    response.days,
                    day_param(&response.zone, event.at),
                    event.id
                ),
                title: texts::title(event),
                who: who(event, copies),
                detail: texts::detail(event),
            }
        })
        .collect();
    rsx! {
        <section class="card st-card">
            <h2>(t("Events in this period"))</h2>
            @if lines.is_empty() {
                <p class="tile-detail">(t("No events."))</p>
            } @else {
                <div class="st-scroll">
                    <table>
                        <thead><tr><th>(when)</th><th>(t("Event"))</th><th>(texts::copy_heading())</th><th>(t("Detail"))</th></tr></thead>
                        <tbody>
                            @for line in &lines {
                                <tr class=(line.class)>
                                    <td class="mono st-nowrap">(&line.when)</td>
                                    <td><a class="st-rowlink" href=(&line.href)><i class=(&line.dot)></i>(&line.title)</a></td>
                                    <td class="st-nowrap tile-detail">(&line.who)</td>
                                    @match &line.detail {
                                        Some((text, true)) => { <td class="mono st-detail">(text)</td> }
                                        Some((text, false)) => { <td class="st-detail">(text)</td> }
                                        None => { <td></td> }
                                    }
                                </tr>
                            }
                        </tbody>
                    </table>
                </div>
            }
        </section>
    }
}
