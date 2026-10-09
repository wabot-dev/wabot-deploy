use hypertext::prelude::*;

use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::entities::tenant::Tenant;
use crate::features::organisations::organisation_quota_component::quota_card;
use crate::features::organisations::texts::t;
use crate::infra::deploy::quotas::Charged;
use crate::web::views::layouts::layout;

/// One organisation, as the node's operator sees it.
///
/// **What is here is what the machine is answerable for**: the name it
/// serves, who answers for it, what it has taken and what it is running.
/// Everything about how the organisation is *run* — its members, which
/// project they reach, what each service is doing — is behind its own
/// console, on its own domain, and is not this page's to show.
pub struct Organisation<'a> {
    pub tenant: &'a Tenant,
    /// Its administrators: whoever the operator handed it to, and
    /// whoever they have made one since.
    pub owners: &'a [Account],
    pub projects: &'a [Project],
    pub charged: &'a Charged,
    pub error: Option<&'a str>,
}

pub fn render<'a>(page: &'a Organisation<'a>) -> impl Renderable + 'a {
    let tenant = page.tenant;
    rsx! {
        (layout::style_tag())
        <div class="split">
            <div class="stack-sm">
                <h1>(&tenant.name)</h1>
                <p class="slug-preview">(&tenant.slug)</p>
            </div>
            <div class="row">
                <a class="btn btn-secondary"
                   href=(format!("/organisations/{}/settings", tenant.id))>(t("Settings"))</a>
            </div>
        </div>
        @if let Some(message) = page.error {
            (layout::error_note(message))
        }
        <section class="card stack">
            <p class="card-label">(t("Console"))</p>
            @if let Some(domain) = &tenant.domain {
                <p class="mono">("https://")(domain)</p>
                <p class="field-hint">(t("Where its people sign in, and the suffix its service \
                     names hang off."))</p>
            } @else {
                <p class="tile-detail">(t("It has no domain, so it has no console yet."))</p>
                <p class="field-hint">(t("Its projects and its people exist; there is nowhere to \
                     administer them from. Give it one in Settings."))</p>
            }
        </section>

        <section class="card stack">
            <p class="card-label">(t("Answers for it"))</p>
            @if page.owners.is_empty() {
                // **Nobody can invite anybody in.** An organisation with
                // no administrator is one the operator has to hand over
                // again, which is the whole reason the Owner tab can
                // reach inside a frontier at all.
                <p class="failure">(t("Nobody. Invite an administrator in Settings — until then \
                     nothing inside this organisation can be changed by anybody."))</p>
            } @else {
                <div class="stack-sm">
                    @for owner in page.owners {
                        <p class="mono">(&owner.username)</p>
                    }
                </div>
                <p class="field-hint">(t("Its administrators. Who else is in it, and what they \
                     reach, is theirs to decide from their own console."))</p>
            }
        </section>

        (quota_card(tenant, page.charged))

        <section class="card stack">
            <p class="card-label">(t("Projects"))</p>
            @if page.projects.is_empty() {
                <p class="tile-detail">(t("None yet. Its administrator makes the first one."))</p>
            } @else {
                // Read-only, and the reason is the rule that orders the
                // whole network: a project is administered from where it
                // was created. What the operator needs is to know what
                // the machine is holding.
                <table>
                    <thead>
                        <tr><th>(t("Project"))</th><th>(t("Slug"))</th></tr>
                    </thead>
                    <tbody>
                        @for project in page.projects {
                            <tr>
                                <td>(&project.name)</td>
                                <td class="mono tile-detail">(&project.slug)</td>
                            </tr>
                        }
                    </tbody>
                </table>
                <p class="field-hint">(t("Listed, not managed. A project is administered from the \
                     console it was made in."))</p>
            }
        </section>
    }
}
