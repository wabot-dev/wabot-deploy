//! A service's tabs, the same on each of its pages.

use hypertext::prelude::*;

use crate::domain::entities::service::Service;
use crate::features::services::service_settings_page::Tab;
use crate::features::services::texts::t;
use crate::shell::components::object_tabs::{object_tabs, ObjectTab};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicePlace {
    Overview,
    Stability,
    Buckets,
    Logs,
    Settings,
}

/// Logs is beside Settings because "what is it saying" is read as often
/// as "how is it configured". Settings goes to the first section this
/// kind of service has: the bare path is a redirect, and a tab that sends
/// somebody through one is a hop nobody asked for.
pub fn service_tabs(
    project_slug: &str,
    service: &Service,
    may_deploy: bool,
    place: ServicePlace,
) -> impl Renderable {
    let base = format!("/projects/{project_slug}/services/{}", service.slug);
    let mut tabs = vec![ObjectTab::new(
        t("Overview"),
        &base,
        place == ServicePlace::Overview,
    )];
    tabs.push(ObjectTab::new(
        t("Stability"),
        format!("{base}/stability"),
        place == ServicePlace::Stability,
    ));
    if service.kind.is_store() {
        tabs.push(ObjectTab::new(
            t("Buckets"),
            format!("{base}/buckets"),
            place == ServicePlace::Buckets,
        ));
    }
    tabs.push(ObjectTab::new(
        t("Logs"),
        format!("{base}/logs"),
        place == ServicePlace::Logs,
    ));
    if may_deploy {
        let settings = match Tab::of(service).first() {
            Some(first) => first.path(project_slug, &service.slug),
            None => format!("{base}/settings"),
        };
        tabs.push(ObjectTab::new(
            t("Settings"),
            settings,
            place == ServicePlace::Settings,
        ));
    }
    let html = object_tabs(&service.name, &tabs).render().into_inner();
    hypertext::Raw::dangerously_create(html)
}
