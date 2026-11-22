use hypertext::prelude::*;

use crate::features::account::texts::t;
use crate::shell::components::object_tabs::{object_tabs, ObjectTab};

/// Where each tab is, in the order they are drawn.
pub fn paths() -> [(&'static str, &'static str); 3] {
    [
        ("passkeys", "/account"),
        ("devices", "/account/devices"),
        ("activity", "/account/activity"),
    ]
}

fn label(tab: &str) -> &'static str {
    match tab {
        "passkeys" => t("Passkeys"),
        "devices" => t("Devices"),
        _ => t("Activity"),
    }
}

/// The account's name and its tabs, the same on every one of them.
pub fn tabs<'a>(current: &'static str, username: &'a str) -> impl Renderable + 'a {
    let tabs: Vec<ObjectTab> = paths()
        .into_iter()
        .map(|(tab, href)| ObjectTab::new(label(tab), href, tab == current))
        .collect();
    let nav = object_tabs(t("Your account"), &tabs).render().into_inner();
    rsx! {
        <div class="stack-sm">
            <h1>(username)</h1>
            <p class="tagline">(t("Your account"))</p>
        </div>
        (hypertext::Raw::dangerously_create(&nav))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::account::Account;
    use crate::shell::layouts::shell::{Frame, Section};

    /// Every tab of the account draws the same frame. `/account` had one
    /// crumb and `/account/devices` two, so a trail appeared between tabs
    /// and pushed the page down by its height (reported from a real node).
    /// Read off the tabs themselves, so a tab added later is covered.
    #[test]
    fn every_tab_of_the_account_draws_the_same_frame() {
        let account = Account {
            theme: crate::domain::values::theme::Theme::System,
            id: "a".into(),
            username: "jorge".into(),
            tenant_id: None,
            role: crate::domain::values::roles::NodeRole::Admin,
            language: crate::domain::values::language::Language::En,
        };
        for (_, path) in paths() {
            let html = Frame::new(&account, Section::Account, path)
                .render(String::new())
                .render()
                .into_inner();
            assert!(!html.contains("class=\"crumbs\""), "{path}: {html}");
        }
    }
}
