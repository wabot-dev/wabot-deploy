//! A project's settings: its push tokens, and deleting it.

use crate::views::layouts::layout;
use crate::views::texts::language::t;
use hypertext::prelude::*;

pub struct ProjectSettings<'a> {
    pub project: &'a crate::entities::project::Project,
    pub allowed: crate::values::roles::Access,
    pub here: &'a str,
    pub members: &'a [crate::repositories::access::Member],
    pub tokens: &'a [crate::entities::push_token::PushToken],
    pub registry_host: Option<&'a str>,
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ProjectSettings<'a>) -> impl Renderable + 'a {
    let project = page.project;
    let allowed = page.allowed;
    let here = page.here;
    let members = page.members;
    let tokens = page.tokens;
    let registry_host = page.registry_host;
    rsx! {
        (layout::style_tag())
        <div class="split">
            <div class="stack-sm">
                <h1>(t("Settings"))</h1>
            </div>
            // What this account may do here, said once at the
            // top. It used to be on the people page and is more
            // use beside the controls it explains: a button
            // that is not there reads as missing until
            // something says why.
            <span class="who">(t("You are: "))(allowed.label())</span>
        </div>
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }

        (crate::features::projects::people_card_component::people_card(here, members, allowed))

        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("Push tokens"))</p>
                <span class="who">(&registry_host)("/")(&project.slug)("/…")</span>
            </div>
            @if tokens.is_empty() {
                <p class="tile-detail">(t("None. A token is what CI authenticates with — it is nobody's \
                     password, and revoking it changes nothing else."))</p>
            } @else {
                <table>
                    <tbody>
                        @for token in tokens {
                            <tr>
                                <td>(&token.name)</td>
                                <td class="tile-detail">
                                    @if token.last_used_at.is_some() {
                                        (t("used"))
                                    } @else {
                                        (t("never used"))
                                    }
                                </td>
                                <td>
                                    <form method="post"
                                          action=(format!(
                                              "{here}/tokens/{}/revoke", token.id
                                          ))>
                                        <button class="btn btn-ghost destructive btn-sm"
                                                type="submit">(t("Revoke"))</button>
                                    </form>
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            }
            <form method="post" action=(format!("{here}/tokens")) class="row">
                <input name="name" type="text" autocomplete="off" placeholder="what it is for" required>
                <button class="btn btn-secondary" type="submit">(t("Create token"))</button>
            </form>
        </section>

        @if allowed.may_administer() {
            <section class="card stack">
                <p class="card-label">(t("Danger zone"))</p>
                // Says what happens rather than what does not.
                // It used to read "Nothing is stopped first — do
                // that yourself", which was true of the code and
                // told somebody to go and do a thing the page
                // gave them no way to do — and left containers
                // running and volumes on disk when they did not.
                <p class="tile-detail">(t("Deleting a project stops and removes every service \
                     under it, and takes everything its databases stored on this node. \
                     There is no undo."))</p>
                // Typing the name is the confirmation — see the
                // handler, which is where it is actually checked.
                <form method="post" action=(format!("{here}/delete")) class="stack">
                    <label for="confirm">(t("Type the name to confirm: "))(&project.slug)</label>
                    <input id="confirm" name="confirm" type="text" autocomplete="off"
                           class="mono" required placeholder=(&project.slug)>
                    <button class="btn btn-danger" type="submit">(t("Delete project"))</button>
                </form>
            </section>
        }
    }
}
