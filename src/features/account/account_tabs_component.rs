use hypertext::prelude::*;

use crate::features::account::texts::t;
use crate::shell::components::object_tabs::{object_tabs, ObjectTab};

pub fn tabs(current: &'static str) -> impl Renderable {
    let tabs = vec![
        ObjectTab::new(t("Passkeys"), "/account", current == "passkeys"),
        ObjectTab::new(t("Devices"), "/account/devices", current == "devices"),
        ObjectTab::new(t("Activity"), "/account/activity", current == "activity"),
    ];
    rsx! {
        (object_tabs(t("Your account"), &tabs))
    }
}
