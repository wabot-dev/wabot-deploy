//! A service's ports: what it listens on, whether that is reachable, and
//! the name it answers to.

use crate::features::services::texts::t;
use hypertext::prelude::*;

/// Declaring a port, as an island host.
///
/// Its own function for the reason `Frame::render` gives: an `rsx!`
/// expands to a closure that captures by move, so nesting one inside
/// another makes both want the same `add` and `suggestion`. Rendering
/// this one first ends its borrow before the page's begins.
/// Where a port can be reached from, in one cell.
/// Where this service answers, split by who can reach it.
///
/// ## Why a list and not a picker
///
/// A database has one thing reachable two ways, so a radio between them
/// is right. A service is not that: **each port is its own exposure**, and
/// a port can be published as raw TCP, served over HTTPS at a hostname,
/// both, or neither. Jorge's question was how somebody sees at a glance
/// what a service offers the world — and a control that shows one string
/// and hides the rest answers the opposite question.
///
/// ## The inside name does not depend on a port
///
/// Every container in the project resolves `<service>.<project>`, because
/// the node writes it into their hosts files from the *replica addresses*
/// — nothing about that reads the port table. A worker that declares no
/// port still has a name, and something else in the project can still
/// reach whatever it listens on. An earlier version of this card offered
/// nothing at all for that service, which was wrong in the direction that
/// matters: it is the case where somebody most needs to be told the name.
pub struct Exposure {
    /// One per way in from outside the node, in the order they were
    /// declared. Empty is a real answer and the card says so.
    pub outside: Vec<String>,
    /// The name a sibling service dials, with a port per declared one —
    /// and the bare name when there are none.
    pub inside: Vec<String>,
}

pub fn port_form<'a>(
    add: &'a str,
    suggestion: &'a Option<(String, bool)>,
    admin: bool,
) -> impl Renderable + 'a {
    wabot::ui::hypertext::island_bare(
        "fields",
        rsx! {
                        <form method="post" action=(add) class="card stack">
                            <label for="container_port">(t("Container port"))</label>
                            <input id="container_port" name="container_port" type="number" autocomplete="off"
                                   min="1" max="65535" placeholder="80" required>
                            <p class="field-hint">(t("What the process listens on inside the container."))</p>

                            <label class="check">
                                <input type="checkbox" name="publish" value="1">
                                (t("Publish on the node's public address (raw TCP)"))
                            </label>
                            // "For a database or anything that is not
                            // HTTP" was right when this form was the only
                            // way to expose one. A managed database has
                            // its own card now and never reaches this
                            // form, so the example named the one case that
                            // no longer arrives here.
                            <p class="field-hint">(t("For anything that is not HTTP — a queue, a \
                                 socket, an engine with its own protocol. The node picks the \
                                 outside port, and it is reachable from the whole internet \
                                 unless a firewall says otherwise."))</p>

                            // Whether HTTPS is offered at all depends on the
                            // node having a domain. Without one, `add_port`
                            // refuses every hostname — it has nothing to
                            // check "does this point here" against — so a
                            // ticked box and a filled field could only ever
                            // come back as an error.
                            @match &suggestion {
                                Some((name, true)) => {
                                    <label class="check">
                                        <input type="checkbox" name="https" value="1">
                                        (t("Serve over HTTPS at a hostname"))
                                    </label>
                                    <div class="stack" data-when="https">
                                        <p class="field-hint">(t("A wildcard record covers this node, so this \
                                             name already resolves here. Leave it as it is."))</p>
                                        <input name="hostname" type="text" autocomplete="off" class="mono"
                                               value=(name) data-required-when="https">
                                    </div>
                                }
                                Some((name, false)) => {
                                    <label class="check">
                                        <input type="checkbox" name="https" value="1">
                                        (t("Serve over HTTPS at a hostname"))
                                    </label>
                                    <div class="stack" data-when="https">
                                        <p class="field-hint">(t("No wildcard record answers for this node, so "))(name)(t(" will not resolve. Either add \
                                             *.<node domain> pointing at this node, or type \
                                             a hostname you have already pointed here — it \
                                             is checked before it is accepted."))</p>
                                        <input name="hostname" type="text" autocomplete="off" class="mono"
                                               placeholder="api.example.com"
                                               data-required-when="https">
                                    </div>
                                }
                                None => {
                                    <label class="check">
                                        <input type="checkbox" name="https" value="1" disabled>
                                        (t("Serve over HTTPS at a hostname"))
                                    </label>
                                    <p class="field-hint">(t("This node has no domain of its own, so it cannot \
                                         check that a name points here — and it will not \
                                         route one it could not check. The node needs a \
                                         domain before anything can be served over HTTPS."))</p>
                                    @if admin {
                                        <a class="btn btn-secondary btn-sm" href="/node">(t("Set the node's domain"))</a>
                                    } @else {
                                        <p class="field-hint">(t("Ask whoever runs this node to set one."))</p>
                                    }
                                }
                            }

                            // **Chosen here rather than after.** A port
                            // created with HTTPS took the default and began
                            // ordering from Let's Encrypt at once, so an
                            // operator who wanted self-signed was already
                            // late for the first attempt — and a failed
                            // order counts against an account that locks at
                            // five an hour.
                            @if suggestion.is_some() {
                                <div class="stack" data-when="https">
                                    <label for="renew_with">(t("Certificate"))</label>
                                    <select id="renew_with" name="renew_with">
                                        <option value="acme">(t("Let's Encrypt"))</option>
                                        <option value="self_signed">(t("Signed here"))</option>
                                        <option value="file">(t("Read from files on this node"))</option>
                                    </select>
                                    <div class="stack" data-when="renew_with=file">
                                        <input name="cert_path" type="text" autocomplete="off"
                                               class="mono" placeholder="/etc/ssl/certs/site.pem"
                                               data-required-when="renew_with=file">
                                        <input name="key_path" type="text" autocomplete="off"
                                               class="mono" placeholder="/etc/ssl/private/site.key"
                                               data-required-when="renew_with=file">
                                    </div>
                                </div>
                            }

                            <div class="actions">
                                <button type="submit">(t("Add port"))</button>
                            </div>
                        </form>
        },
    )
}

pub fn exposure(
    service: &crate::domain::entities::service::Service,
    project_slug: &str,
    ports: &[crate::domain::entities::port::Port],
    node_domain: Option<&str>,
) -> Exposure {
    let name = format!("{}.{project_slug}", service.slug);
    let mut outside = Vec::new();

    for port in ports {
        // A hostname is served over HTTPS by an edge; a published port is
        // whatever the container speaks, on this node's address. A port
        // can be both, and then it is two ways in rather than one.
        if let Some(hostname) = &port.hostname {
            outside.push(format!("https://{hostname}"));
        }
        if let Some(host_port) = port.host_port {
            outside.push(match node_domain {
                Some(domain) => format!("{domain}:{host_port}"),
                // Without a domain the number is still worth showing: the
                // operator knows their own address and this is the part
                // they cannot guess.
                None => format!("this node:{host_port}"),
            });
        }
    }

    let inside = match ports.is_empty() {
        true => vec![name],
        false => ports
            .iter()
            .map(|port| format!("{name}:{}", port.container_port))
            .collect(),
    };
    Exposure { outside, inside }
}

/// Both lists, each line copyable.
///
/// Every string on screen at once, which is the point. The copy island
/// puts a button on each — it walks `[data-copy]` and appends one per
/// block, so a list costs nothing that a single string did not.
pub fn exposure_card<'a>(exposure: &'a Exposure, running: bool) -> impl Renderable + 'a {
    let (copy, copied) = (t("Copy"), t("Copied"));
    let line = move |value: &str| {
        rsx! {
            <div class="dsn-line">
                <pre class="dsn-value" data-copy
                     data-copy-label=(copy) data-copied-label=(copied)>(value)</pre>
            </div>
        }
        .render()
        .into_inner()
    };
    let outside: Vec<String> = exposure.outside.iter().map(|url| line(url)).collect();
    let inside: Vec<String> = exposure.inside.iter().map(|url| line(url)).collect();

    let inner = rsx! {
        <div class="dsn stack-sm">
            <p class="card-label">(t("From outside the node"))</p>
            @if outside.is_empty() {
                <p class="field-hint">(t("Nothing. It answers inside the project only — publish \
                     a port or give one a hostname in settings."))</p>
            } @else {
                <div class="dsn-values">
                    @for html in &outside {
                        (hypertext::Raw::dangerously_create(html))
                    }
                </div>
            }

            <p class="card-label">(t("Inside the project"))</p>
            <div class="dsn-values">
                @for html in &inside {
                    (hypertext::Raw::dangerously_create(html))
                }
            </div>
            <p class="field-hint">(t("Every container in this project resolves that name, on \
                 any node holding a copy — whether or not anything is exposed."))</p>
            @if !running {
                <p class="field-hint">(t("Nothing answers at any of these while it is not \
                     running."))</p>
            }
        </div>
    }
    .render()
    .into_inner();

    // XSS SAFETY: every line above is this function's own markup, built
    // from strings the page already renders escaped.
    wabot::ui::hypertext::island(
        "copy",
        &serde_json::json!({}),
        hypertext::Raw::dangerously_create(&inner),
    )
}

pub fn reachable_at(
    port: &crate::domain::entities::port::Port,
    node_domain: Option<&str>,
) -> String {
    let mut where_from = Vec::new();
    if let Some(hostname) = &port.hostname {
        where_from.push(format!("https://{hostname}"));
    }
    if let Some(host_port) = port.host_port {
        where_from.push(match node_domain {
            Some(domain) => format!("{domain}:{host_port}"),
            None => format!("this node:{host_port}"),
        });
    }
    if where_from.is_empty() {
        // Not "nothing": the port is reachable, just only from inside
        // the project, and that is a useful thing to have said.
        return "the project only".into();
    }
    where_from.join("  ")
}
