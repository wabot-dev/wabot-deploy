//! Whether a service stayed within itself: day by day, one day up close,
//! and what happened in between.

use hypertext::prelude::*;

use crate::domain::entities::copy_metric::BUCKET_MS;
use crate::domain::entities::service::Service;
use crate::domain::values::language::Language;
use crate::features::services::service_tabs_component::{service_tabs, ServicePlace};
use crate::features::stability::see_service_stability_request::day_param;
use crate::features::stability::service_stability_response::ServiceStabilityResponse;
use crate::features::stability::stability_chart_component::Plot;
use crate::features::stability::stability_day_component::day;
use crate::features::stability::stability_events_component::events;
use crate::features::stability::stability_strip_component::strip;
use crate::features::stability::texts::{self, t};
use crate::shell::layouts::layout;

pub struct StabilityPage<'a> {
    pub project_slug: &'a str,
    pub service: &'a Service,
    pub may_deploy: bool,
    pub response: &'a ServiceStabilityResponse,
}

impl StabilityPage<'_> {
    pub fn path(&self) -> String {
        format!(
            "/projects/{}/services/{}/stability",
            self.project_slug, self.service.slug
        )
    }
}

/// What the island needs to answer a pointer without asking the node: the
/// numbers each chart was drawn from and the words around them.
pub fn island_data(page: &StabilityPage, plots: &[Plot]) -> serde_json::Value {
    let response = page.response;
    serde_json::json!({
        "start": response.selected,
        "step": BUCKET_MS,
        "buckets": response.buckets,
        "comma": crate::shell::texts::language::current() == Language::Es,
        "copies": response
            .copies
            .iter()
            .map(|copy| texts::copy(copy.slot, copy.node.as_deref()))
            .collect::<Vec<_>>(),
        "words": { "none": t("no data"), "ceiling": t("ceiling") },
        "charts": plots.iter().map(|plot| plot.data.clone()).collect::<Vec<_>>(),
        "events": response
            .day_events
            .iter()
            .map(|event| serde_json::json!({
                "at": event.at,
                "until": event.until,
                "title": texts::title(event),
            }))
            .collect::<Vec<_>>(),
    })
}

pub fn render<'a>(page: &'a StabilityPage<'a>, plots: &'a [Plot]) -> impl Renderable + 'a {
    let response = page.response;
    let base = page.path();
    let periods: Vec<(bool, String, String)> = [30u32, 90]
        .into_iter()
        .map(|days| {
            (
                response.days == days,
                format!("{base}?days={days}&day={}", day_param(response.selected)),
                texts::days(days),
            )
        })
        .collect();
    let unreachable: Vec<String> = response
        .unreachable
        .iter()
        .map(|(node, reason)| texts::not_asked(node, reason))
        .collect();
    rsx! {
        (layout::style_tag())
        <h1>(&page.service.name)</h1>
        (service_tabs(page.project_slug, page.service, page.may_deploy, ServicePlace::Stability))
        <nav class="st-seg" aria-label=(t("Period"))>
            @for (on, href, label) in &periods {
                @if *on {
                    <span class="on" aria-current="true">(label)</span>
                } @else {
                    <a href=(href)>(label)</a>
                }
            }
        </nav>
        @for sentence in &unreachable {
            <p class="form-warning">(sentence)</p>
        }
        (strip(response, &base))
        (day(response, plots, &base))
        (events(response, &base))
    }
}
