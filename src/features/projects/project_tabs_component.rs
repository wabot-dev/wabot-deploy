//! A project's tabs, the same on each of its pages.

use hypertext::prelude::*;

use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::projects::texts::t;
use crate::shell::components::object_tabs::{object_tabs, ObjectTab};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectPlace {
    Overview,
    Dns,
    Settings,
}

/// Settings is offered to whoever may change something there. A tab
/// somebody would be turned away from is the page lying about what they
/// can do; the check that counts is still on each POST.
pub fn project_tabs(project: &Project, allowed: Access, place: ProjectPlace) -> impl Renderable {
    let base = format!("/projects/{}", project.slug);
    let mut tabs = vec![
        ObjectTab::new(t("Overview"), &base, place == ProjectPlace::Overview),
        ObjectTab::new(
            t("DNS"),
            format!("{base}/zones"),
            place == ProjectPlace::Dns,
        ),
    ];
    if allowed.may_deploy() {
        tabs.push(ObjectTab::new(
            t("Settings"),
            format!("{base}/settings"),
            place == ProjectPlace::Settings,
        ));
    }
    let html = object_tabs(&project.name, &tabs).render().into_inner();
    hypertext::Raw::dangerously_create(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::roles::ProjectRole;

    fn tabs(allowed: Access, place: ProjectPlace) -> String {
        let project = Project {
            id: "prj-1".into(),
            name: "Shop".into(),
            slug: "shop".into(),
            created_at: 0,
            origin_node_id: None,
            tenant_id: None,
        };
        project_tabs(&project, allowed, place).render().into_inner()
    }

    /// Somebody who may look and change nothing is not offered the tab
    /// that would turn them away. Its page redirects them; a tab that
    /// leads to a redirect is the page lying about what they can do.
    #[test]
    fn settings_is_offered_to_whoever_may_change_something() {
        let viewer = tabs(Access::member(ProjectRole::Viewer), ProjectPlace::Overview);
        assert!(
            viewer.contains(r#"href="/projects/shop/zones""#),
            "{viewer}"
        );
        assert!(!viewer.contains("/projects/shop/settings"), "{viewer}");

        let admin = tabs(Access::ADMIN, ProjectPlace::Settings);
        assert!(
            admin.contains(r#"href="/projects/shop/settings" class="active""#),
            "{admin}"
        );
    }
}
