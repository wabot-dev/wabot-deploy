//! **This machine's own operators, and nobody else's people.**
//!
//! An organisation's are managed from its own console, by a different
//! page. This one knows nothing about organisations, which is what lets
//! that whole half be deleted without touching it.

use hypertext::prelude::*;

use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::ProjectRole;
use crate::infra::accounts::invitations::Invitation;
use crate::web::views::layouts::layout;
use crate::web::views::texts::language::t;

pub struct People<'a> {
    pub account: &'a Account,
    pub people: &'a [Account],
    pub invitations: &'a [Invitation],
    pub projects: &'a [Project],
    pub now: i64,
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a People<'a>) -> impl Renderable + 'a {
    let account = page.account;
    let people = page.people;
    let invitations = page.invitations;
    let projects = page.projects;
    let now = page.now;
    rsx! {
    (layout::style_tag())
    <h1>(t("People"))</h1>
    @if let Some(message) = &page.query_error {
        (layout::error_note(message))
    }

    <section class="stack">
        <p class="card-label">(t("Accounts"))</p>
        <table>
            <thead>
                <tr><th>(t("Name"))</th><th>(t("On this node"))</th><th></th></tr>
            </thead>
            <tbody>
                @for person in people {
                    <tr>
                        <td>(&person.username)</td>
                        <td>(person.role.label())</td>
                        <td class="row">
                            @if person.id != account.id {
                                <form method="post"
                                      action=(format!("/people/{}/role", person.id))>
                                    <input type="hidden" name="role" value=(
                                        if person.is_admin() { "member" } else { "admin" }
                                    )>
                                    <button class="btn btn-secondary btn-sm" type="submit">
                                        @if person.is_admin() {
                                            (t("Make a member"))
                                        } @else {
                                            (t("Make an administrator"))
                                        }
                                    </button>
                                </form>
                                <form method="post"
                                      action=(format!("/people/{}/delete", person.id))>
                                    <button class="btn btn-ghost destructive btn-sm"
                                            type="submit">
                                        (t("Remove"))
                                    </button>
                                </form>
                            } @else {
                                <span class="tile-detail">(t("This is you"))</span>
                            }
                        </td>
                    </tr>
                }
            </tbody>
        </table>
    </section>

    <section class="stack">
        <p class="card-label">(t("Invite somebody"))</p>
        <form method="post" action="/people/invite" class="card stack">
            <label for="node_role">(t("On this node"))</label>
            <select id="node_role" name="node_role">
                <option value="member">(t("Member — their own projects"))</option>
                <option value="admin">(t("Administrator — everything"))</option>
            </select>

            <label for="project">(t("Into a project"))</label>
            <select id="project" name="project">
                <option value="">(t("Nothing yet"))</option>
                @for project in projects {
                    <option value=(&project.slug)>(&project.name)</option>
                }
            </select>

            <label for="project_role">(t("As"))</label>
            <select id="project_role" name="project_role">
                @for role in ProjectRole::ALL {
                    <option value=(role.as_str())>(role.label())</option>
                }
            </select>
            <p class="field-hint">(t("They choose their own username and password. Nobody here \
                 ever sees it — which is the reason this is a link rather \
                 than a form that sets one for them."))</p>

            <div class="actions">
                <button type="submit">(t("Create invitation"))</button>
            </div>
        </form>
    </section>

    <section class="stack">
        <p class="card-label">(t("Invitations"))</p>
        @if invitations.is_empty() {
            <p class="tile-detail">(t("None."))</p>
        } @else {
            <table>
                <thead>
                    <tr><th>(t("For"))</th><th>(t("State"))</th><th></th></tr>
                </thead>
                <tbody>
                    @for invitation in invitations {
                        <tr>
                            <td>
                                (invitation.node_role.label())
                                @if let Some(name) = &invitation.scope_name {
                                    <div class="tile-detail">
                                        (t("Organisation")) " " (name)
                                    </div>
                                }
                            </td>
                            <td>
                                @if invitation.spent() {
                                    <span class="badge">(t("Used"))</span>
                                } @else if invitation.expired(now) {
                                    <span class="badge badge-warning">(t("Expired"))</span>
                                } @else {
                                    <span class="badge badge-success">
                                        <span class="dot dot-success"></span>(t("Open"))
                                    </span>
                                }
                            </td>
                            <td>
                                @if !invitation.spent() {
                                    <form method="post"
                                          action=(format!(
                                              "/people/invitations/{}/revoke",
                                              invitation.id
                                          ))>
                                        <button class="btn btn-ghost destructive btn-sm"
                                                type="submit">
                                            (t("Withdraw"))
                                        </button>
                                    </form>
                                }
                            </td>
                        </tr>
                    }
                </tbody>
            </table>
        }
    </section>
    }
}
