use hypertext::prelude::*;

use crate::domain::entities::allowlist_set::AllowlistSet;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::project_settings_tabs_component::{
    project_settings_tabs, ProjectSettingsTab,
};
use crate::features::projects::project_tabs_component::{project_tabs, ProjectPlace};
use crate::features::projects::texts::t;
use crate::shell::layouts::layout;

pub struct ProjectAllowlists<'a> {
    pub project: &'a Project,
    pub allowed: Access,
    pub sets: &'a [(AllowlistSet, Vec<String>)],
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ProjectAllowlists<'a>) -> impl Renderable + 'a {
    let project = page.project;
    let base = format!("/projects/{}/allowlists", project.slug);
    rsx! {
        (layout::style_tag())
        <div class="split">
            <div class="stack-sm">
                <h1>(&project.name)</h1>
            </div>
            <span class="who">(t("You are: "))(page.allowed.label())</span>
        </div>
        (project_tabs(project, page.allowed, ProjectPlace::Settings))
        (project_settings_tabs(&project.slug, ProjectSettingsTab::Allowlists))
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }

        <p class="tile-detail">(t("Addresses several services let in — an office, a VPN, a CDN's \
             ranges — kept once. A service uses a list on its own Who may connect tab, and lets in \
             its own lines and those of every list it uses."))</p>

        @for (set, users) in page.sets {
            <section class="card stack" id=(format!("list-{}", set.name))>
                <div class="split">
                    <p class="card-label mono">(&set.name)</p>
                    @if users.is_empty() {
                        <span class="who">(t("No service uses it"))</span>
                    } @else {
                        <span class="who">(t("Used by "))(users.join(", "))</span>
                    }
                </div>
                <form method="post" action=(format!("{base}/{}", set.name)) class="stack">
                    <textarea name="allowlist" autocomplete="off" rows="4" class="mono"
                              aria-label=(t("Addresses and ranges"))
                              placeholder="203.0.113.5 office">(set.list.text())</textarea>
                    <p class="field-hint">(t("One address or range per line, and a note after it if \
                         you like. Saving changes every service using it at once, with no \
                         deployment."))</p>
                    <div class="actions">
                        <button type="submit">(t("Save"))</button>
                    </div>
                </form>
                @if users.is_empty() {
                    <form method="post" action=(format!("{base}/{}/delete", set.name))>
                        <button class="btn btn-ghost destructive btn-sm" type="submit">(t("Delete list"))</button>
                    </form>
                }
            </section>
        }

        <section class="card stack" id="new-list">
            <p class="card-label">(t("New list"))</p>
            <form method="post" action=(base) class="stack">
                <label for="list-name">(t("Name"))</label>
                <input id="list-name" name="name" type="text" autocomplete="off" class="mono"
                       required placeholder="office">
                <label for="list-lines">(t("Addresses and ranges"))</label>
                <textarea id="list-lines" name="allowlist" autocomplete="off" rows="4" class="mono"
                          placeholder="203.0.113.5 office"></textarea>
                <div class="actions">
                    <button type="submit">(t("Create list"))</button>
                </div>
            </form>
        </section>
    }
}
