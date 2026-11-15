//! The tabs of one thing: a node, a project, a service, an organisation.
//!
//! Every one of them reads the same way — what it is first, what is
//! particular to it in the middle, its settings last — so somebody who
//! has opened one knows where to look in the next.

use hypertext::prelude::*;

pub struct ObjectTab {
    pub label: &'static str,
    pub href: String,
    pub current: bool,
}

impl ObjectTab {
    pub fn new(label: &'static str, href: impl Into<String>, current: bool) -> Self {
        Self {
            label,
            href: href.into(),
            current,
        }
    }
}

pub fn object_tabs<'a>(name: &'a str, tabs: &'a [ObjectTab]) -> impl Renderable + 'a {
    rsx! {
        <nav class="object-tabs" aria-label=(name)>
            @for tab in tabs {
                @if tab.current {
                    <a href=(&tab.href) class="active" aria-current="page">(tab.label)</a>
                } @else {
                    <a href=(&tab.href)>(tab.label)</a>
                }
            }
        </nav>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One tab is where you are, and it says so to a reader that cannot
    /// see which is raised.
    #[test]
    fn the_current_tab_is_marked_once() {
        let tabs = [
            ObjectTab::new("Overview", "/nodes/nd-1", false),
            ObjectTab::new("Settings", "/nodes/nd-1/settings", true),
        ];
        let html = object_tabs("node-1", &tabs).render().into_inner();
        assert_eq!(html.matches("aria-current=\"page\"").count(), 1, "{html}");
        assert!(
            html.contains(r#"<a href="/nodes/nd-1/settings" class="active""#),
            "{html}"
        );
    }
}
