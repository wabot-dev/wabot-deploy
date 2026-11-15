//! An organisation's tabs, as whoever runs the machine sees them.
//!
//! Two, because that is what the operator has of one: what the machine
//! is holding for it, and the four things they decide about it. How it is
//! run is behind its own console.

use hypertext::prelude::*;

use crate::domain::entities::tenant::Tenant;
use crate::features::organisations::node_user_organisation_settings_page::OrganisationTab;
use crate::features::organisations::texts::t;
use crate::shell::components::object_tabs::{object_tabs, ObjectTab};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrganisationPlace {
    Overview,
    Settings,
}

pub fn organisation_tabs(tenant: &Tenant, place: OrganisationPlace) -> impl Renderable {
    let tabs = [
        ObjectTab::new(
            t("Overview"),
            format!("/organisations/{}", tenant.id),
            place == OrganisationPlace::Overview,
        ),
        ObjectTab::new(
            t("Settings"),
            OrganisationTab::Domain.path(&tenant.id),
            place == OrganisationPlace::Settings,
        ),
    ];
    let html = object_tabs(&tenant.name, &tabs).render().into_inner();
    hypertext::Raw::dangerously_create(html)
}
