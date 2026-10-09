use hypertext::prelude::*;

use crate::domain::entities::config_revision::Revision;
use crate::domain::entities::port::Port;
use crate::domain::entities::project::Project;
use crate::domain::entities::replica::Replica;
use crate::domain::entities::service::Service;
use crate::features::databases::database_cards_component;
use crate::features::services::service_push_component::Push;
use crate::features::services::texts::t;
use crate::features::services::{
    service_placement_component, service_ports_component, service_push_component,
};
use crate::infra::edge::policy::Policy;
use crate::infra::network::errand::Record;
use crate::infra::network::Node;
use crate::shell::components::node_certificate::{source_word, CertificateCells};
use crate::shell::layouts::layout;

/// Which tab of a service's settings a page is showing.
///
/// **One URL each**, rather than one page that hides and shows with
/// script. A tab can be linked, opened in another window and read with
/// scripting off, and the page sends only the sections it is showing —
/// which is the actual cause of the scroll this replaced. Twelve sections
/// in a column meant the way to reach the last one was to know it was
/// there.
///
/// The set depends on the kind, and not as a courtesy: a managed database
/// has no image, no tag and no environment — the node writes all three
/// from the engine's row — and no ports, edges or certificates of the sort
/// this console serves, because Postgres speaks its own protocol on a
/// published port. Offering it those tabs would be offering five empty
/// pages. A tab it does not have is redirected rather than rendered
/// blank, so a hand-typed URL lands somewhere real.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Image,
    Database,
    Environment,
    Resources,
    Network,
    Replicas,
    Logs,
    Danger,
}

impl Tab {
    /// The path segment. Not translated: it is a URL, and a URL that
    /// changes with the reader's language is one that cannot be shared.
    pub fn slug(self) -> &'static str {
        match self {
            Tab::Image => "image",
            Tab::Database => "database",
            Tab::Environment => "environment",
            Tab::Resources => "resources",
            Tab::Network => "network",
            Tab::Replicas => "replicas",
            Tab::Logs => "logs",
            Tab::Danger => "danger",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Tab::Image => t("Image"),
            Tab::Database => t("Database"),
            Tab::Environment => t("Environment"),
            Tab::Resources => t("Resources"),
            Tab::Network => t("Network"),
            Tab::Replicas => t("Replicas"),
            Tab::Logs => t("Logs"),
            Tab::Danger => t("Danger"),
        }
    }

    pub fn parse(slug: &str) -> Option<Self> {
        [
            Tab::Image,
            Tab::Database,
            Tab::Environment,
            Tab::Resources,
            Tab::Network,
            Tab::Replicas,
            Tab::Logs,
            Tab::Danger,
        ]
        .into_iter()
        .find(|tab| tab.slug() == slug)
    }

    /// The tabs this service has, in the order they are shown.
    pub fn of(service: &Service) -> Vec<Tab> {
        let mut tabs = match service.kind.is_managed() {
            true => vec![Tab::Database, Tab::Resources],
            false => vec![Tab::Image, Tab::Environment, Tab::Resources, Tab::Network],
        };
        // Where the copies go is a decision only the node that placed
        // them can make. A service that arrived on an errand has none to
        // offer, so the tab would be a page with nothing on it.
        if service.is_ours() {
            tabs.push(Tab::Replicas);
        }
        // **Including a database.** This was gated to plain containers
        // because the logs section happened to sit inside the branch that
        // also held the image, the tag and the environment — three things
        // the node writes for a database. Timestamping is not one of them:
        // `try_deploy` reads `service.log_timestamps` for every kind and
        // hands containerd the same `binary://` either way, so the setting
        // already worked and only the page withheld it.
        tabs.push(Tab::Logs);
        tabs.push(Tab::Danger);
        tabs
    }

    /// Where a save on this tab comes back to.
    pub fn path(self, project: &str, service: &str) -> String {
        format!(
            "/projects/{project}/services/{service}/settings/{}",
            self.slug()
        )
    }
}

/// One tab's worth, and only that tab's.
///
/// Three of the eight need nothing but the service, so they are units:
/// what they draw is a form over columns the row already carries. The
/// other five name exactly what they read, which is what stopped the
/// handler fetching all of it for whichever one was asked for.
pub enum Panel {
    Database {
        /// The first name it answers to. `None` before it has one, which
        /// is the state between creation and the first deployment.
        name: Option<String>,
        /// Where each of the three forms posts, in the order they are
        /// drawn.
        actions: (String, String, String),
        /// Both or neither: the card needs the policy to say what would
        /// renew it and the cells to say what is installed, and one
        /// without the other is a card that can only be half wrong.
        certificate: Option<(Policy, CertificateCells)>,
        host_port: Option<u16>,
    },
    Image {
        push: Option<Push>,
        /// Where the push tokens are, which is the project's page and not
        /// this one.
        project_settings: String,
    },
    Logs,
    Environment {
        env_text: String,
        history: Vec<Revision>,
    },
    Resources,
    Network {
        ports: Vec<Port>,
        /// Per port: its id, the name, what is installed and what would
        /// renew it. Read once for the page rather than per hostname.
        certificates: Vec<(String, String, CertificateCells, Policy)>,
        /// The names with a certificate from a real authority, so the
        /// "on the way" badge is hidden for the ones that have arrived.
        secured: std::collections::BTreeSet<String>,
        domain: Option<String>,
        /// Where a new port posts, and where removing one does.
        add: String,
        suggestion: Option<(String, bool)>,
        /// Whether the reader may publish a port on the node's own
        /// address. A courtesy — the check that counts is on the POST.
        may_publish: bool,
        /// Whether this node asks an authority at all, which decides the
        /// certificate a new hostname starts with.
        acme_off: bool,
        /// The nodes that could answer for a name, those that were asked,
        /// and the instructions still waiting to be collected.
        public_nodes: Vec<Node>,
        serving: Vec<(String, String)>,
        orders: Vec<Record>,
    },
    Replicas {
        placements: Vec<Replica>,
        nodes: Vec<Node>,
        silent: Vec<String>,
        queued: Vec<String>,
    },
    Danger,
}

pub struct ServiceSettings<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    /// The service's own path, which every form on every tab posts under.
    pub base: &'a str,
    pub tabs: &'a [Tab],
    pub tab: Tab,
    pub panel: Panel,
    pub error: Option<&'a str>,
    pub checked: Option<&'a str>,
}

pub fn render<'a>(page: &'a ServiceSettings<'a>) -> impl Renderable + 'a {
    let (project, service, base) = (page.project, page.service, page.base);
    rsx! {
        (layout::style_tag())
        <div class="split">
            <div class="stack-sm">
                <h1>(t("Settings"))</h1>
            </div>
            // No "Back to service" here either: the breadcrumb above
            // already goes there. Third time this pattern was reported, so
            // there is now a test that fails on the fourth — see
            // `no_page_offers_a_way_back_the_breadcrumb_already_is`.
        </div>

        // Links, not buttons and not script: each tab is a page.
        // `aria-current` rather than only a class, because "which one am I
        // on" is the whole information a tab bar carries and a colour is
        // not available to everybody reading it.
        <nav class="tabs" aria-label=(t("Settings"))>
            @for entry in page.tabs {
                @if *entry == page.tab {
                    <a href=(entry.path(&project.slug, &service.slug))
                       class="tab active" aria-current="page">(entry.label())</a>
                } @else {
                    <a href=(entry.path(&project.slug, &service.slug))
                       class="tab">(entry.label())</a>
                }
            }
        </nav>

        @if let Some(message) = page.error {
            (layout::error_note(message))
        }
        @if let Some(message) = page.checked {
            <p class="note">(message)</p>
        }

        @match &page.panel {
            // A managed database has no image to point at, no tag to watch
            // and no environment to edit: the node writes all three from
            // the engine's row, and a form here would be one whose value
            // the next deployment overwrites. What is left to choose is
            // the size.
            Panel::Database { name, actions, certificate, host_port } => {
                @if let Some(name) = name {
                    (database_cards_component::name_card(
                        &actions.0, name, &crate::infra::deploy::hosts::pool_name(name)))
                }
                @if let Some((policy, cells)) = certificate {
                    (database_cards_component::certificate_card(&actions.1, policy, cells))
                }
                @if let Some(name) = name {
                    (database_cards_component::published_card(&actions.2, *host_port, name))
                }
            }
            Panel::Image { push, project_settings } => {
                // **The image first, then what happens on a push.** The
                // tracking form used to be above, so the tab opened on a
                // rule about pushes before saying what the service runs.
                // Asked for in this order by Jorge.
                <section class="stack" id="image">
                    <p class="card-label">(t("Image"))</p>
                    // The form that did not exist. A service was pinned to
                    // whatever it was created with, and this card's own
                    // push example told people to "point the service at
                    // this reference" with nothing to point it with.
                    // Reported by Jorge.
                    <form method="post" action=(format!("{base}/image")) class="card stack">
                        <input id="service-image" name="image" type="text" autocomplete="off"
                               class="mono" value=(&service.image)
                               placeholder="docker.io/library/nginx:alpine">
                        <p class="field-hint">(t("Saving redeploys the service with this image. \
                             Empty makes it wait for a push to its own repository \
                             instead."))</p>
                        <div class="actions">
                            <button type="submit">(t("Save image"))</button>
                        </div>
                    </form>
                </section>

                <section class="stack" id="releases">
                    <p class="card-label">(t("Automatic deployment"))</p>
                    <form method="post" action=(format!("{base}/tracking")) class="card stack">
                        // The switch before the field it governs: the tag
                        // only means anything once a push is allowed to
                        // deploy, and the form read the other way round.
                        <label class="check">
                            // `checked="false"` checks it. This box read as
                            // on whatever the row said, and saving the form
                            // then turned it on for real.
                            @if service.auto_deploy {
                                <input type="checkbox" name="auto_deploy" value="1" checked>
                            } @else {
                                <input type="checkbox" name="auto_deploy" value="1">
                            }
                            (t("Deploy a push automatically"))
                        </label>
                        <p class="field-hint">(t("Without this, a push is recorded as a release and waits \
                             for somebody to deploy it from the service page."))</p>

                        <label for="track_tag">(t("Tag to watch"))</label>
                        <input id="track_tag" name="track_tag" type="text" autocomplete="off" class="mono"
                               value=(service.track_tag.clone().unwrap_or_default())
                               placeholder="latest">
                        <p class="field-hint">(t("Pushing to this tag deploys the image \
                             immediately. You can use * to deploy automatically whatever you \
                             push."))</p>
                        <div class="actions">
                            <button type="submit">(t("Save"))</button>
                        </div>
                    </form>

                    // What to type, on the page that decides what a push
                    // does. The service's own page has the whole sequence
                    // behind a disclosure; this is the one command, with
                    // the name in full, because the question here is "what
                    // do I push to".
                    (service_push_component::push_target(push, project_settings))
                </section>
            }
            Panel::Logs => {
                <section class="stack" id="logs">
                    <p class="card-label">(t("Logs"))</p>
                    <form method="post" action=(format!("{base}/log-timestamps")) class="card stack">
                        <label class="check">
                            @if service.log_timestamps {
                                <input type="checkbox" name="log_timestamps" value="1" checked>
                            } @else {
                                <input type="checkbox" name="log_timestamps" value="1">
                            }
                            (t("Timestamp every line, and mark which stream it came from"))
                        </label>
                        // **The cost, in numbers measured on a node.** This
                        // is a switch whose price is invisible until
                        // somebody goes looking for history that is no
                        // longer there, so the figures are on the form
                        // rather than in a document.
                        <p class="field-hint">(t("Without this, containerd appends the container's \
                             bytes to a file and no process sits in between. With it, one \
                             wabot-deploy process per copy reads the output and writes it \
                             — which is what makes a timestamp possible, and what the \
                             costs below buy."))</p>
                        <ul class="field-hint">
                            <li>(t("Memory: about 0.3 MB of private memory per copy. `top` will show nearer 8 MB, and almost all of that is this binary's own code shared with every other wabot-deploy process on the machine — including the node itself. Measured here, on a real service."))</li>
                            <li>(t("Disk: 28 bytes on every line. On a real service here that is a fifth more, so the same retention budget holds about a sixth fewer lines — timestamps are paid for in history."))</li>
                            <li>(t("Risk: the reader is between the container and its log. If it stops reading, the container blocks on its next write. It drops lines rather than stopping, which is the whole reason this is offered at all — but it is why the default is off."))</li>
                        </ul>
                        <p class="field-hint">(t("Takes effect at the next deployment: containerd cannot change a running container's output."))</p>

                        // **Bytes, because the question is disk.** The
                        // per-container rule was a *count* of generations,
                        // so "how much may this service keep" had no answer
                        // and no dial — and the two services on a node that
                        // matter are never the two that talk the same
                        // amount.
                        <label for="log_budget">(t("Keep at most"))</label>
                        <input id="log_budget" name="log_budget" type="text" autocomplete="off"
                               class="mono"
                               placeholder=(t("two rotations"))
                               value=(service.log_budget
                                   .map(crate::domain::policies::presets::size_field)
                                   .unwrap_or_default())>
                        <p class="field-hint">(t("Per copy, and the live log counts against it \
                             — a service with two copies on this machine writes two. Oldest \
                             history is dropped first, and what the container is saying now \
                             is never dropped."))</p>
                        // **What "two rotations" weighs, said rather than
                        // left to be asked.** Jorge asked, and the
                        // constant's own comment answered it wrongly — see
                        // `logs::GENERATIONS`. A rotation is not a fixed
                        // size: it is whatever the live file had reached at
                        // the restart that rotated it.
                        <p class="field-hint">(t("Empty keeps two rotations behind the live \
                             file. No log file passes 8 MB, so that is 24 MB for a copy \
                             that has restarted twice."))</p>
                        <p class="field-hint">(t("The node's own budget still applies over \
                             all of them, so this can ask for more than it gets."))</p>
                        <div class="actions">
                            <button type="submit">(t("Save"))</button>
                        </div>
                    </form>
                </section>
            }
            Panel::Environment { env_text, history } => {
                <section class="stack" id="environment">
                    <p class="card-label">(t("Environment"))</p>
                    <form method="post" action=(format!("{base}/env")) class="card stack">
                        <textarea name="env" autocomplete="off" rows="6" class="mono"
                                  placeholder="KEY=value">(env_text)</textarea>
                        <p class="field-hint">(t("Saving redeploys the service with these values. The image \
                             it runs does not change."))</p>
                        <div class="actions">
                            <button type="submit">(t("Save environment"))</button>
                        </div>
                    </form>

                    @if !history.is_empty() {
                        <table>
                            <tbody>
                                @for revision in history.iter().skip(1) {
                                    <tr>
                                        // **What restoring this would
                                        // leave.** The row said only how
                                        // many values there were and the
                                        // raw word `edit`, so choosing
                                        // between two revisions meant
                                        // guessing — and the button
                                        // replaces the running
                                        // environment. Reported by Jorge.
                                        //
                                        // A `<details>` rather than a
                                        // second page: it is native, so it
                                        // opens with scripting off, and it
                                        // keeps a long history readable.
                                        // The values are shown in full
                                        // because the textarea above
                                        // already shows the current ones
                                        // that way — hiding only the old
                                        // ones would protect nothing.
                                        <td class="tile-detail">
                                            <details>
                                                <summary>
                                                    <span class="mono">(layout::exactly(revision.created_at))</span>
                                                    (" · ")
                                                    @if revision.env.len() == 1 {
                                                        (t("1 value"))
                                                    } @else {
                                                        (revision.env.len())(t(" values"))
                                                    }
                                                    (" · ")
                                                    @if revision.reason == "revert" {
                                                        (t("restored"))
                                                    } @else {
                                                        (t("edited"))
                                                    }
                                                </summary>
                                                @if revision.env.is_empty() {
                                                    <p>(t("No values — restoring this clears the environment."))</p>
                                                } @else {
                                                    <pre class="mono">(
                                                        revision.env.iter()
                                                            .map(|(key, value)| format!("{key}={value}"))
                                                            .collect::<Vec<_>>()
                                                            .join("\n")
                                                    )</pre>
                                                }
                                            </details>
                                        </td>
                                        <td>
                                            <form method="post"
                                                  action=(format!("{base}/env/{}/revert", revision.id))>
                                                <button class="btn btn-ghost btn-sm"
                                                        type="submit">(t("Restore"))</button>
                                            </form>
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    }
                </section>
            }
            // What a container may take, for every kind of service.
            //
            // The memory half used to be a database's alone, because that
            // is where presets were invented — and a plain container could
            // take the machine's memory with nothing on any page to stop
            // it. The column was always on `service`; only the form was
            // not.
            //
            // The section carries the id, because the id is where a save
            // comes back to. The memory field's own was `memory` too, and
            // two elements with one id is a fragment that lands on
            // whichever the browser finds first.
            Panel::Resources => {
                <section class="stack" id="memory">
                    <p class="card-label">(t("Resources"))</p>
                    <form method="post" action=(format!("{base}/memory")) class="card stack">
                        <label for="memory-size">(t("Memory"))</label>
                        // A preset for a managed engine, because the number
                        // also sets the engine's own arithmetic; anything
                        // else takes a size.
                        @if service.kind.is_managed() {
                            <select id="memory-size" name="memory">
                                <option value="">(t("no ceiling"))</option>
                                @for rung in crate::domain::policies::presets::LADDER {
                                    @if Some(rung) == service.memory_limit {
                                        <option value=(rung.to_string()) selected>(
                                            crate::domain::policies::presets::label(rung)
                                        )</option>
                                    } @else {
                                        <option value=(rung.to_string())>(
                                            crate::domain::policies::presets::label(rung)
                                        )</option>
                                    }
                                }
                            </select>
                        } @else {
                            <input id="memory-size" name="memory" type="text"
                                   list="memory-sizes" autocomplete="off"
                                   placeholder=(t("no ceiling"))
                                   value=(service.memory_limit
                                       .map(crate::domain::policies::presets::size_field)
                                       .unwrap_or_default())>
                            <datalist id="memory-sizes">
                                @for rung in crate::domain::policies::presets::LADDER {
                                    <option value=(
                                        crate::domain::policies::presets::size_field(rung)
                                    )></option>
                                }
                            </datalist>
                        }
                        @if service.kind.is_managed() {
                            <p class="field-hint">(t("The ceiling on the container and the engine's \
                                 own settings, together. It takes effect at the next deployment: a \
                                 cgroup limit is written when the container is created, and nothing \
                                 reaches into a running one to change it."))</p>
                        } @else {
                            <p class="field-hint">(t("A size, like 512 MB or 1.5 GB — a bare \
                                 number is megabytes. Over this the kernel kills the container \
                                 rather than letting it swap, which is the outcome to want: a \
                                 process quietly swapping is invisible until everything on the \
                                 node is slow."))</p>
                        }

                        <label for="cpu-size">(t("CPU"))</label>
                        <input id="cpu-size" name="cpu" type="text"
                               list="cpu-sizes" autocomplete="off"
                               placeholder=(t("no ceiling"))
                               value=(service.cpu_millicores
                                   .map(crate::domain::policies::presets::cores_field)
                                   .unwrap_or_default())>
                        <datalist id="cpu-sizes">
                            @for rung in crate::domain::policies::presets::CPU_LADDER {
                                <option value=(
                                    crate::domain::policies::presets::cores_field(rung)
                                )></option>
                            }
                        </datalist>
                        <p class="field-hint">(t("Cores, like 0.5 or 2 — or millicores with an \
                             m, like 500m. Over this the container is throttled, not killed: it \
                             runs slowly rather than stopping. A ceiling is also what is reserved \
                             for it — the node counts what it has promised, and refuses to \
                             promise more than it has."))</p>

                        <label for="disk-size">(t("Disk"))</label>
                        <input id="disk-size" name="disk" type="text"
                               autocomplete="off"
                               placeholder=(t("no ceiling"))
                               value=(service.disk_limit
                                   .map(crate::domain::policies::presets::label)
                                   .unwrap_or_default())>
                        <p class="field-hint">(t("This one reserves space; it does not enforce it. \
                             There is nothing to write it into — volumes share a filesystem, \
                             and dividing one properly needs XFS project quotas or a file per \
                             volume. What the node does with this number is refuse to place \
                             more than it can hold, and say on this page when a copy has gone \
                             past what it was promised."))</p>
                        <div class="actions">
                            <button type="submit">(t("Save"))</button>
                        </div>
                    </form>
                </section>
            }
            Panel::Network {
                ports, certificates, secured, domain, add, suggestion, may_publish,
                acme_off, public_nodes, serving, orders,
            } => {
                <section class="stack" id="ports">
                    <p class="card-label">(t("Exposed ports"))</p>
                    @if ports.is_empty() {
                        <p class="tile-detail">(t("This service exposes nothing. That is the right answer for \
                             a worker; add a port for anything that listens."))</p>
                    } @else {
                        <table>
                            <thead>
                                <tr>
                                    <th>(t("Port"))</th>
                                    <th>(t("Type"))</th>
                                    <th>(t("Reachable at"))</th>
                                    <th>(t("Certificate"))</th>
                                    <th></th>
                                </tr>
                            </thead>
                            <tbody>
                                @for port in ports {
                                    @let kind = match (port.host_port.is_some(),
                                                       port.hostname.is_some()) {
                                        (true, true) => t("HTTPS · TCP"),
                                        (false, true) => t("HTTPS"),
                                        (true, false) => t("TCP"),
                                        (false, false) => t("internal"),
                                    };
                                    @let certificate = certificates
                                        .iter()
                                        .find(|(id, ..)| id == &port.id);
                                    <tr>
                                        <td class="mono">(port.container_port)</td>
                                        <td>(kind)</td>
                                        <td class="mono reach">
                                            <span>(service_ports_component::reachable_at(port, domain.as_deref()))</span>
                                            @if let Some(hostname) = &port.hostname {
                                                <span data-name=(hostname)
                                                      class=(match secured.contains(hostname) {
                                                          true => "badge badge-info is-hidden",
                                                          false => "badge badge-info",
                                                      })>
                                                    <span class="dot dot-info dot-pulse"></span>
                                                    (t("Certificate on the way"))
                                                </span>
                                            }
                                        </td>
                                        <td>
                                            @if let Some((_, _, cells, policy)) = certificate {
                                                <span class=(cells.badge)>
                                                    <span class=(cells.dot)></span>
                                                    <span>(cells.word)</span>
                                                </span>
                                                <span class="tile-detail">(
                                                    source_word(&policy.renew_with)
                                                )</span>
                                            } @else {
                                                <span class="tile-detail">("—")</span>
                                            }
                                        </td>
                                        <td>
                                            <form method="post"
                                                  action=(format!("{add}/{}/delete", port.id))>
                                                <button class="btn btn-ghost destructive btn-sm"
                                                        type="submit">
                                                    (t("Remove"))
                                                </button>
                                            </form>
                                        </td>
                                    </tr>
                                    @if let Some((_, _, cells, _)) = certificate {
                                        @if !cells.failure.is_empty() {
                                            <tr class="port-detail">
                                                <td colspan="5">
                                                    <p class="failure">(&cells.failure)</p>
                                                </td>
                                            </tr>
                                        }
                                    }
                                }
                            </tbody>
                        </table>
                    }

                    <p class="card-label">(t("Expose a port"))</p>
                    (service_ports_component::port_form(add, suggestion, *may_publish, domain.is_some(), *acme_off))
                </section>

                @if service.is_ours() && !public_nodes.is_empty()
                    && ports.iter().any(|port| port.hostname.is_some()) {
                    <section class="stack" id="edges">
                        <p class="card-label">(t("Nodes that expose the service"))</p>
                        <div class="card stack">
                            @for port in ports {
                                @if let Some(hostname) = &port.hostname {
                                    (service_placement_component::served_by_form(
                                        project, service, hostname, public_nodes, serving, orders))
                                }
                            }
                            <p class="field-hint">(t("A node answers for a name by claiming it, \
                                 getting a certificate for it, and proxying to wherever the \
                                 copies run. Only nodes with an address the world can dial \
                                 are here, and only those that agreed to be asked."))</p>
                        </div>
                    </section>
                }
            }
            Panel::Replicas { placements, nodes, silent, queued } => {
                @if service.is_ours() {
                    (service_placement_component::placement_card(
                        project, service, placements, nodes, silent, queued))
                }
            }
            Panel::Danger => {
                <section class="card stack">
                    <p class="card-label">(t("Danger zone"))</p>
                    @if service.kind.is_managed() {
                        <p class="tile-detail">(t("Deleting a database stops it and removes everything \
                             it stored on this node. There is no undo and there is no backup — a \
                             read-only copy is not one, because a deletion reaches it too."))</p>
                    } @else {
                        <p class="tile-detail">(t("Deleting a service stops its container and removes it. The \
                             images it was built from stay in the registry."))</p>
                    }
                    <form method="post" action=(format!("{base}/delete")) class="stack">
                        <label for="confirm">(t("Type the name to confirm: "))(&service.slug)</label>
                        <input id="confirm" name="confirm" type="text" autocomplete="off"
                               class="mono" required placeholder=(&service.slug)>
                        @if service.kind.is_managed() {
                            <button class="btn btn-danger" type="submit">(t("Delete database and its data"))</button>
                        } @else {
                            <button class="btn btn-danger" type="submit">(t("Delete service"))</button>
                        }
                    </form>
                </section>
            }
        }
    }
}
