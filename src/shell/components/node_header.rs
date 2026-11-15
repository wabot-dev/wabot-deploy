//! A node's name and its tabs, the same on every page that is about it.
//!
//! Drawn by three features — the network's pages, this machine's
//! settings and its releases — which is why it is the shell's.

use hypertext::prelude::*;

use crate::infra::network::Node;
use crate::shell::components::object_tabs::{object_tabs, ObjectTab};
use crate::shell::texts::language::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodePlace {
    Overview,
    Connections,
    Version,
    Settings,
}

/// Where each tab is.
///
/// This machine's settings and releases are at `/node`, which names
/// nobody: it is the same address on every node, so a concern, a
/// document and `doctor` can all point at it without knowing an id.
/// Another node's are under its own.
pub fn node_tabs(node: &Node, place: NodePlace) -> Vec<ObjectTab> {
    let base = format!("/nodes/{}", node.id);
    let (version, settings) = match node.is_self {
        true => ("/node/version".to_string(), "/node/domain".to_string()),
        false => (format!("{base}/version"), format!("{base}/settings")),
    };
    vec![
        ObjectTab::new(t("Overview"), &base, place == NodePlace::Overview),
        ObjectTab::new(
            t("Connections"),
            format!("{base}/connections"),
            place == NodePlace::Connections,
        ),
        ObjectTab::new(t("Version"), version, place == NodePlace::Version),
        ObjectTab::new(t("Settings"), settings, place == NodePlace::Settings),
    ]
}

pub fn node_header(node: &Node, place: NodePlace) -> impl Renderable + '_ {
    let tabs = node_tabs(node, place);
    let tabs = object_tabs(&node.name, &tabs).render().into_inner();
    let version = match node.is_self {
        true => Some(crate::infra::update::VERSION.to_string()),
        false => node.version.clone(),
    };
    rsx! {
        <div class="stack-sm">
            <div class="row">
                <h1>(&node.name)</h1>
                @if node.is_self {
                    <span class="badge">(t("this node"))</span>
                }
            </div>
            @if let Some(version) = &version {
                <p class="slug-preview">("wabot-deploy ")(version)</p>
            }
        </div>
        (hypertext::Raw::dangerously_create(&tabs))
    }
}
