use hypertext::prelude::*;

use crate::accounts::invitations::Invitation;
use crate::accounts::roles::NodeRole;
use crate::accounts::roles::ProjectRole;
use crate::accounts::Account;
use crate::entities::project::Project;
use crate::views::layouts::layout;
use crate::views::texts::language::t;

pub struct People<'a> {
    pub people: &'a [Account],
    pub live: &'a [&'a Invitation],
    pub projects: &'a [Project],
    pub account: &'a Account,
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a People<'a>) -> impl Renderable + 'a {
    let People {
        people,
        live,
        projects,
        account,
        query_error,
    } = page;

    rsx! {
        (layout::style_tag())
        <h1>(t("People"))</h1>
        <p class="tagline">(t("Everybody in this organisation. What they can reach is its \
             projects and nothing else on this machine."))</p>

        @if let Some(message) = query_error {
            (layout::error_note(message))
        }

        <section class="stack">
            <p class="card-label">(t("Accounts"))</p>
            <table>
                <thead>
                    <tr>
                        <th>(t("Name"))</th>
                        <th>(t("In this organisation"))</th>
                        <th></th>
                    </tr>
                </thead>
                <tbody>
                    @for person in *people {
                        <tr>
                            <td>(&person.username)</td>
                            <td>
                                @if person.administers_scope() {
                                    (t("Administrator"))
                                } @else {
                                    (t("Member"))
                                }
                            </td>
                            <td class="row">
                                @if person.id != account.id {
                                    <form method="post"
                                          action=(format!("/people/{}/role", person.id))>
                                        <input type="hidden" name="role" value=(
                                            if person.administers_scope() { "member" } else { "admin" }
                                        )>
                                        <button class="btn btn-secondary btn-sm" type="submit">
                                            @if person.administers_scope() {
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
                <label for="node_role">(t("In this organisation"))</label>
                <select id="node_role" name="node_role">
                    <option value="member">(t("Member — the projects they are added to"))</option>
                    <option value="admin">(t("Administrator — every project of this organisation"))</option>
                </select>

                @if !projects.is_empty() {
                    <label for="project">(t("Into a project"))</label>
                    <select id="project" name="project">
                        <option value="">(t("Nothing yet"))</option>
                        @for project in *projects {
                            <option value=(&project.slug)>(&project.name)</option>
                        }
                    </select>

                    <label for="project_role">(t("As"))</label>
                    <select id="project_role" name="project_role">
                        @for role in ProjectRole::ALL {
                            <option value=(role.as_str())>(crate::views::texts::language::word(role.label()))</option>
                        }
                    </select>
                }
                <p class="field-hint">(t("A link that works once and expires in seven days. \
                     They choose their own username and password."))</p>
                <div class="actions">
                    <button type="submit">(t("Create invitation"))</button>
                </div>
            </form>
        </section>

        @if !live.is_empty() {
            <section class="stack">
                <p class="card-label">(t("Invitations waiting"))</p>
                <table>
                    <thead>
                        <tr><th>(t("In this organisation"))</th><th></th></tr>
                    </thead>
                    <tbody>
                        @for invitation in *live {
                            <tr>
                                <td>
                                    @if invitation.node_role == NodeRole::Admin {
                                        (t("Administrator"))
                                    } @else {
                                        (t("Member"))
                                    }
                                </td>
                                <td>
                                    <form method="post" action=(
                                        format!("/people/invitations/{}/revoke", invitation.id)
                                    )>
                                        <button class="btn btn-ghost destructive btn-sm"
                                                type="submit">
                                            (t("Withdraw"))
                                        </button>
                                    </form>
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            </section>
        }
    }
}
