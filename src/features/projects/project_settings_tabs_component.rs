use hypertext::prelude::*;

use crate::features::projects::texts::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectSettingsTab {
    General,
    Environment,
    Allowlists,
    Internet,
}

impl ProjectSettingsTab {
    const ALL: [ProjectSettingsTab; 4] = [
        ProjectSettingsTab::General,
        ProjectSettingsTab::Environment,
        ProjectSettingsTab::Allowlists,
        ProjectSettingsTab::Internet,
    ];

    pub fn path(self, project: &str) -> String {
        match self {
            ProjectSettingsTab::General => format!("/projects/{project}/settings"),
            ProjectSettingsTab::Environment => format!("/projects/{project}/settings/environment"),
            ProjectSettingsTab::Allowlists => format!("/projects/{project}/settings/allowlists"),
            ProjectSettingsTab::Internet => format!("/projects/{project}/settings/internet"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            ProjectSettingsTab::General => t("General"),
            ProjectSettingsTab::Environment => t("Shared environment"),
            ProjectSettingsTab::Allowlists => t("Shared allowlists"),
            ProjectSettingsTab::Internet => t("Internet"),
        }
    }
}

pub fn project_settings_tabs(project: &str, current: ProjectSettingsTab) -> impl Renderable {
    let html = rsx! {
        <nav class="tabs" aria-label=(t("Settings"))>
            @for entry in ProjectSettingsTab::ALL {
                @if entry == current {
                    <a href=(entry.path(project)) class="tab active" aria-current="page">(entry.label())</a>
                } @else {
                    <a href=(entry.path(project)) class="tab">(entry.label())</a>
                }
            }
        </nav>
    }
    .render()
    .into_inner();
    hypertext::Raw::dangerously_create(html)
}
