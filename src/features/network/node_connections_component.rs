//! What is happening between this node and the others.
//!
//! Two facts beside each other and never folded into one verdict: what
//! the kernel says about the tunnel, and when that node last reported.
//! Either can be old while the other is fresh — a node that reports to
//! this one need not be exchanging packets with it — and a single green
//! dot would be a health check this console does not run.

use hypertext::prelude::*;

use crate::features::network::texts::t;
use crate::infra::network::tunnel::{Paired, Path};
use crate::infra::network::Node;
use crate::shell::layouts::layout;

/// A tunnel renews its handshake within three minutes while packets
/// cross it, so an older one means nothing has since.
const QUIET_AFTER_SECONDS: u64 = 180;

/// The other nodes, each with what is known of the tunnel to it.
pub struct Links<'a> {
    pub nodes: &'a [Node],
    /// The kernel's answer, or why it could not be read.
    pub paths: &'a Result<Vec<Paired>, String>,
}

impl Links<'_> {
    fn others(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|node| !node.is_self)
    }

    pub fn path_to(&self, node: &Node) -> Option<&Path> {
        self.paths
            .as_ref()
            .ok()?
            .iter()
            .find(|paired| paired.node_id == node.id)
            .map(|paired| &paired.path)
    }
}

/// The badge and the detail for one tunnel.
///
/// The class, then the word, then what goes beside it: an address, or
/// how long ago. `None` is an interface that could not be read, which is
/// not the same as a tunnel that is down.
pub fn tunnel_words(path: Option<&Path>) -> (&'static str, &'static str, Option<String>) {
    match path {
        Some(Path::Session { ago, at }) => {
            let when = layout::when(crate::shell::state::now_ms() - (*ago as i64) * 1000);
            let detail = match at {
                Some(at) => format!("{when} · {at}"),
                None => when,
            };
            match *ago <= QUIET_AFTER_SECONDS {
                true => ("success", t("Connected"), Some(detail)),
                false => ("warning", t("Quiet"), Some(detail)),
            }
        }
        Some(Path::Dialling { at }) => ("danger", t("No answer"), Some(at.clone())),
        Some(Path::Waiting) => ("info", t("Waiting for it to dial in"), None),
        Some(Path::Nowhere) => ("warning", t("No path"), None),
        Some(Path::NotOnTheOverlay) => ("info", t("Not on the overlay yet"), None),
        Some(Path::Unexplained { .. }) | None => ("info", t("Unknown"), None),
    }
}

pub fn tunnel_badge(path: Option<&Path>) -> impl Renderable {
    let (tone, word, _) = tunnel_words(path);
    rsx! {
        <span class=(format!("badge badge-{tone}"))>
            <span class=(format!("dot dot-{tone}"))></span>(word)
        </span>
    }
}

/// Every other node, one row each.
pub fn connections_card<'a>(links: &'a Links<'a>) -> impl Renderable + 'a {
    rsx! {
        <div class="stack">
            <p class="card-label">(t("Connections with other nodes"))</p>
            @if links.others().next().is_none() {
                <div class="card stack">
                    <p class="tile-detail">(t("This node is on its own. Nothing has joined it, \
                         and it has joined nothing."))</p>
                </div>
            } @else {
                <div class="card stack">
                    // Five columns do not fit a phone, and a table that
                    // pushes the page sideways takes the bar at the
                    // bottom with it: it scrolls inside its own box.
                    <div class="scroll-x">
                        <table>
                            <thead>
                                <tr>
                                    <th>(t("Node"))</th><th>(t("Tunnel"))</th>
                                    <th>(t("Last handshake"))</th><th>(t("Last report"))</th>
                                    <th>(t("Version"))</th>
                                </tr>
                            </thead>
                            <tbody>
                                @for node in links.others() {
                                    (connection_row(node, links.path_to(node)))
                                }
                            </tbody>
                        </table>
                    </div>
                    (unreadable_note(links.paths))
                    <p class="field-hint">(t("The tunnel is what the kernel says, read when this \
                         page was drawn. A handshake is renewed every couple of minutes while \
                         packets cross, so a quiet one has carried nothing since — a node that \
                         is off, or one with nothing to say. A report is that node telling this \
                         one how it is, which only a node that joined through this one does."))</p>
                </div>
            }
        </div>
    }
}

fn connection_row<'a>(node: &'a Node, path: Option<&'a Path>) -> impl Renderable + 'a {
    let detail = tunnel_words(path).2.unwrap_or_else(|| "—".to_string());
    rsx! {
        <tr>
            <td><a href=(format!("/nodes/{}", node.id))>(&node.name)</a></td>
            <td>(tunnel_badge(path))</td>
            <td class="tile-detail">(&detail)</td>
            <td class="tile-detail">
                (node.last_seen_at.map(layout::when).unwrap_or_else(|| "—".to_string()))
            </td>
            <td class="mono">(node.version.as_deref().unwrap_or("—"))</td>
        </tr>
    }
}

/// The one tunnel a page about another node is about.
pub fn connection_card<'a>(node: &'a Node, links: &'a Links<'a>) -> impl Renderable + 'a {
    let path = links.path_to(node);
    let detail = tunnel_words(path).2.unwrap_or_else(|| "—".to_string());
    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("Between this node and it"))</p>
                (tunnel_badge(path))
            </div>
            <dl class="kv">
                <dt>(t("Last handshake"))</dt>
                <dd>(&detail)</dd>
                <dt>(t("Last report"))</dt>
                <dd>(node.last_seen_at.map(layout::when).unwrap_or_else(|| "—".to_string()))</dd>
            </dl>
            (unreadable_note(links.paths))
        </section>
    }
}

fn unreadable_note(paths: &Result<Vec<Paired>, String>) -> impl Renderable + '_ {
    rsx! {
        @if let Err(reason) = paths {
            <p class="note">(t("The overlay could not be read on this machine, so the tunnels \
                 are unknown: "))(reason)</p>
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A handshake three hours old is a session by the kernel's account
    /// and not a connection by anybody's. The two get different words,
    /// and neither is the green one unless packets crossed just now.
    #[test]
    fn an_old_handshake_is_not_called_connected() {
        let fresh = Path::Session {
            ago: 40,
            at: Some("203.0.113.9:51820".into()),
        };
        let old = Path::Session {
            ago: 3 * 60 * 60,
            at: None,
        };
        assert_eq!(tunnel_words(Some(&fresh)).1, "Connected");
        assert_eq!(tunnel_words(Some(&fresh)).0, "success");
        assert_eq!(tunnel_words(Some(&old)).1, "Quiet");
        assert_eq!(tunnel_words(Some(&old)).0, "warning");
        assert!(tunnel_words(Some(&old))
            .2
            .is_some_and(|detail| detail.contains("3 hours ago")));
    }

    /// An interface nobody could read is not a tunnel that is down: the
    /// page says it does not know, and why.
    #[test]
    fn an_unreadable_overlay_is_unknown_and_says_why() {
        let nodes = [Node {
            update_note: None,
            version: None,
            wanted_version: None,
            usage: None,
            draining_at: None,
            capacity: None,
            id: "nd-other".into(),
            name: "node-1".into(),
            kind: crate::infra::network::Kind::Private,
            endpoint: None,
            public_key: None,
            overlay_ip: Some("10.42.0.2".into()),
            is_self: false,
            last_seen_at: None,
            allows: Vec::new(),
            ca_pem: None,
        }];
        let paths = Err("the overlay needs a Linux kernel".to_string());
        let html = connections_card(&Links {
            nodes: &nodes,
            paths: &paths,
        })
        .render()
        .into_inner();
        assert!(html.contains("Unknown"), "{html}");
        assert!(html.contains("the overlay needs a Linux kernel"), "{html}");
        assert!(!html.contains("badge-danger"), "{html}");
        assert!(html.contains(r#"href="/nodes/nd-other""#), "{html}");
    }

    /// The only fault among them is the one where this node believes it
    /// can reach that one and never has.
    #[test]
    fn only_a_peer_that_never_answered_is_red() {
        let dialling = Path::Dialling {
            at: "203.0.113.9:51820".into(),
        };
        assert_eq!(tunnel_words(Some(&dialling)).0, "danger");
        for path in [Path::Waiting, Path::Nowhere, Path::NotOnTheOverlay] {
            assert_ne!(tunnel_words(Some(&path)).0, "danger", "{path:?}");
        }
    }
}
