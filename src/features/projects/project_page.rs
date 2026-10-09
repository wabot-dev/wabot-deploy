//! One project: what runs in it, and what needs attention.

use crate::views::layouts::layout;
use crate::views::texts::language::t;
use hypertext::prelude::*;

pub struct ProjectView<'a> {
    pub project: &'a crate::entities::project::Project,
    pub allowed: crate::values::roles::Access,
    pub rows: &'a [(
        crate::entities::service::Service,
        crate::features::services::service_state_component::StateCell,
        Option<String>,
    )],
    pub new_service: &'a str,
    pub new_database: &'a str,
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ProjectView<'a>) -> impl Renderable + 'a {
    let project = page.project;
    let allowed = page.allowed;
    let rows = page.rows;
    let new_service = page.new_service;
    let new_database = page.new_database;
    rsx! {
            (layout::style_tag())
            <div class="split">
                <div class="stack-sm">
                    <h1>(&project.name)</h1>
                </div>
                @if allowed.may_deploy() {
                    <div class="row">
                        // Secondary, and beside rather than
                        // above: a database is a service of a
                        // particular kind, not a second thing
                        // this page is about.
                        <a class="btn btn-secondary" href=(&new_database)>(
                            t("Create database")
                        )</a>
                        <a class="btn" href=(&new_service)>(t("Create service"))</a>
                    </div>
                }
            </div>
            @if let Some(message) = &page.query_error {
                (layout::error_note(message))
            }

            @if rows.is_empty() {
                // No second pair of buttons here. The ones in the
                // header are on this page whether or not the list
                // is empty, so an empty project showed both a
                // hand's width apart — which reads as two
                // different things rather than one. The projects
                // list above already had this right; this page
                // did not follow it. Reported by Jorge.
                <section class="empty">
                    <p>(t("No services yet."))</p>
                </section>
            } @else {
                (crate::features::projects::service_table_component::service_table(project, rows, allowed))
            }

    }
}
