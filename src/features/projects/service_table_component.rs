//! What a project's services are doing, as a table.
//!
//! One row per service, and the state is a coloured dot plus a word —
//! never a colour alone.

use hypertext::prelude::*;

use crate::features::services::service_state_component::StateCell;
use crate::web::views::texts::language::t;

/// What a service is doing, from containerd rather than from a column.
///
/// The distinction the badge exists to make: "Running" is a task with a
/// pid, and everything else says which kind of not-running it is. A
/// runtime that cannot be reached is its own answer, because
/// redeploying is the wrong response to it.
/// The services table, as an island host.
///
/// Its own function for the reason `Frame::render` gives: an `rsx!`
/// expands to a closure that captures by move, so nesting one inside
/// the page's would have both wanting the same `project` and `rows`.
/// A reference moves freely — it is `Copy` — and the markup is
/// rendered before the host wraps it.
pub fn service_table(
    project: &crate::domain::entities::project::Project,
    // The third is why the copy running here is not, when it is not.
    // Carried alongside rather than read off the service: it belongs to
    // a replica now, and a service is several of them.
    rows: &[(
        crate::domain::entities::service::Service,
        StateCell,
        Option<String>,
    )],
    allowed: crate::domain::values::roles::Access,
) -> impl Renderable {
    let inner = rsx! {
                    <table>
                        <thead>
                            <tr>
                                <th>(t("Service"))</th>
                                // The copies. Headed by nothing, for the
                                // reason the delete column is: the cell
                                // says "3 replicas · 2 elsewhere", which
                                // names itself. It was headed "Address"
                                // and held one, until a service had more
                                // than one copy and the column started
                                // answering a different question.
                                <th class="address"></th>
                                <th class="state">(t("State"))</th>
                                // The delete column. Headed by nothing,
                                // because "Delete" over a column of
                                // Delete buttons reads as a heading for
                                // a thing rather than a label for one.
                                <th></th>
                            </tr>
                        </thead>
                        <tbody>
                            @for (service, cell, failure) in rows {
                                <tr>
                                    <td>
                                        <a href=(format!(
                                            "/projects/{}/services/{}",
                                            project.slug, service.slug
                                        ))>(&service.name)</a>
                                    </td>
                                    <td class="mono address" data-address=(&service.id)>
                                        (cell.whereabouts.say())
                                    </td>
                                    <td class="state" data-state=(&service.id)>
                                        (crate::features::services::service_state_component::state_badge(cell))
                                    </td>
                                    <td class="row">
                                        @if allowed.may_deploy() {
                                            (crate::features::services::service_state_component::deploy_controls(
                                                &project.slug,
                                                &service.slug,
                                                cell,
                                                &format!("/projects/{}", project.slug),
                                            ))
                                        }
                                    </td>
                                </tr>
                                @if let Some(failure) = failure {
                                    <tr>
                                        <td colspan="4" class="failure">(failure)</td>
                                    </tr>
                                }
                            }
                        </tbody>
                    </table>
    }
    .render()
    .into_inner();

    wabot::ui::hypertext::island(
        "project-live",
        &serde_json::json!({ "project": &project.slug }),
        hypertext::Raw::dangerously_create(&inner),
    )
}
