use hypertext::prelude::*;

use crate::domain::entities::env_group::EnvGroup;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::project_settings_tabs_component::{
    project_settings_tabs, ProjectSettingsTab,
};
use crate::features::projects::project_tabs_component::{project_tabs, ProjectPlace};
use crate::features::projects::texts::t;
use crate::shell::layouts::layout;

pub struct ProjectEnvGroups<'a> {
    pub project: &'a Project,
    pub allowed: Access,
    pub groups: &'a [(EnvGroup, Vec<String>)],
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ProjectEnvGroups<'a>) -> impl Renderable + 'a {
    let project = page.project;
    let base = format!("/projects/{}/env-groups", project.slug);
    rsx! {
        (layout::style_tag())
        <div class="split">
            <div class="stack-sm">
                <h1>(&project.name)</h1>
            </div>
            <span class="who">(t("You are: "))(page.allowed.label())</span>
        </div>
        (project_tabs(project, page.allowed, ProjectPlace::Settings))
        (project_settings_tabs(&project.slug, ProjectSettingsTab::Environment))
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }

        <p class="tile-detail">(t("Variables several services read — an SMTP account, an error \
             tracker's key — kept once. A service joins a group on its own Environment tab, and a \
             variable of its own wins over the same one in a group."))</p>

        @for (group, users) in page.groups {
            <section class="card stack" id=(format!("group-{}", group.name))>
                <div class="split">
                    <p class="card-label mono">(&group.name)</p>
                    @if users.is_empty() {
                        <span class="who">(t("No service has joined it"))</span>
                    } @else {
                        <span class="who">(t("Joined by "))(users.join(", "))</span>
                    }
                </div>
                <form method="post" action=(format!("{base}/{}", group.name)) class="stack">
                    <textarea name="env" autocomplete="off" rows="5" class="mono"
                              aria-label=(t("Variables"))
                              placeholder="KEY=value">(
                        group.env.iter()
                            .map(|(key, value)| format!("{key}={value}\n"))
                            .collect::<String>()
                    )</textarea>
                    <p class="field-hint">(t("Saving redeploys every running service that joined it, \
                         with the release it is on."))</p>
                    <div class="actions">
                        <button type="submit">(t("Save"))</button>
                    </div>
                </form>
                @if users.is_empty() {
                    <form method="post" action=(format!("{base}/{}/delete", group.name))>
                        <button class="btn btn-ghost destructive btn-sm" type="submit">(t("Delete group"))</button>
                    </form>
                }
            </section>
        }

        <section class="card stack" id="new-group">
            <p class="card-label">(t("New group"))</p>
            <form method="post" action=(base) class="stack">
                <label for="group-name">(t("Name"))</label>
                <input id="group-name" name="name" type="text" autocomplete="off" class="mono"
                       required placeholder="smtp">
                <label for="group-env">(t("Variables"))</label>
                <textarea id="group-env" name="env" autocomplete="off" rows="5" class="mono"
                          placeholder="SMTP_HOST=mail.example.com"></textarea>
                <div class="actions">
                    <button type="submit">(t("Create group"))</button>
                </div>
            </form>
        </section>
    }
}
