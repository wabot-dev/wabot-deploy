use hypertext::prelude::*;

use crate::domain::entities::project::Project;
use crate::domain::values::destinations::Destinations;
use crate::domain::values::roles::Access;
use crate::features::projects::project_settings_tabs_component::{
    project_settings_tabs, ProjectSettingsTab,
};
use crate::features::projects::project_tabs_component::{project_tabs, ProjectPlace};
use crate::features::projects::texts::t;
use crate::infra::repositories::egress::Refused;
use crate::shell::layouts::layout;

pub struct ProjectEgress<'a> {
    pub project: &'a Project,
    pub allowed: Access,
    pub destinations: &'a Destinations,
    pub exits: &'a [ExitChoice],
    pub refused: &'a [Refused],
    pub query_error: Option<&'a str>,
}

/// A node the project may leave through, or one it was told to that no
/// longer lets it.
pub struct ExitChoice {
    pub node_id: Option<String>,
    pub name: String,
    pub is_self: bool,
    pub chosen: bool,
}

pub fn render<'a>(page: &'a ProjectEgress<'a>) -> impl Renderable + 'a {
    let project = page.project;
    let destinations = page.destinations;
    let editable = page.allowed.may_administer() && project.is_ours();
    rsx! {
        (layout::style_tag())
        <div class="split">
            <div class="stack-sm">
                <h1>(&project.name)</h1>
            </div>
            <span class="who">(t("You are: "))(page.allowed.label())</span>
        </div>
        (project_tabs(project, page.allowed, ProjectPlace::Settings))
        (project_settings_tabs(&project.slug, ProjectSettingsTab::Internet))
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }

        <section class="card stack" id="destinations">
            <p class="card-label">(t("Where it may connect"))</p>
            @if destinations.is_empty() {
                <p class="note">(t("Nothing leaves: the list is empty."))</p>
            } @else if destinations.is_anywhere() {
                <p class="field-hint">(t("It may connect anywhere."))</p>
            } @else {
                <p class="field-hint">(t("Only the destinations below."))</p>
            }
            @if editable {
                <form method="post" action=(format!("/projects/{}/egress", project.slug)) class="stack">
                    <textarea name="destinations" autocomplete="off" rows="6" class="mono"
                              aria-label=(t("Destinations"))
                              placeholder="203.0.113.5:443 payments">(destinations.text())</textarea>
                    <p class="field-hint">(t("One destination per line: an address, a range, or a \
                         name like api.example.com or *.example.com, a port after a colon if it is \
                         one port, and a note after it if you like. Empty lets nothing out. \
                         Anywhere is 0.0.0.0/0."))</p>
                    <p class="field-hint">(t("A name opens the addresses it resolves to, and with \
                         them every other name served from those addresses: a name on a CDN opens \
                         the CDN's. Any other name does not resolve, unless the list reaches every \
                         address on some port."))</p>
                    <p class="field-hint">(t("Its own services, its names on other nodes and this \
                         node's own addresses are reached whatever the list says. A connection \
                         already open stays open until it closes."))</p>
                    <div class="actions">
                        <button type="submit">(t("Save"))</button>
                    </div>
                </form>
            } @else {
                <pre class="mono">(destinations.text())</pre>
                @if !project.is_ours() {
                    <p class="field-hint">(t("Administered from the node that sent it."))</p>
                }
            }
        </section>

        <section class="card stack" id="exits">
            <p class="card-label">(t("Leaves through"))</p>
            @if !page.exits.iter().any(|exit| exit.chosen) {
                <p class="field-hint">(t("Each copy leaves from the node it runs on."))</p>
            }
            @if editable && page.exits.iter().any(|exit| exit.node_id.is_some()) {
                <form method="post" action=(format!("/projects/{}/exits", project.slug)) class="stack">
                    @for exit in page.exits {
                        @if let Some(node_id) = &exit.node_id {
                            <label class="capability">
                                @if exit.chosen {
                                    <input type="checkbox" name=(format!("exit-{node_id}")) value="1" checked>
                                } @else {
                                    <input type="checkbox" name=(format!("exit-{node_id}")) value="1">
                                }
                                <span class="mono">(&exit.name)</span>
                                @if exit.is_self {
                                    <span class="tile-detail">(t("this node"))</span>
                                }
                            </label>
                        }
                    }
                    <p class="field-hint">(t("Its connections to the internet leave from the chosen \
                         nodes' addresses, a different one each time, over TCP: anything else \
                         leaving for the internet is dropped. When none of them answers, nothing \
                         leaves. None chosen is each copy from its own node."))</p>
                    <p class="field-hint">(t("A node is listed once it lets this one name it as an \
                         exit. Private addresses and this project's own network are reached from \
                         the node a copy runs on."))</p>
                    <div class="actions">
                        <button type="submit">(t("Save"))</button>
                    </div>
                </form>
            } @else if editable {
                <p class="field-hint">(t("No other node lets this one name it as an exit."))</p>
            }
            @for exit in page.exits.iter().filter(|exit| exit.chosen && (exit.node_id.is_none() || !editable)) {
                <p>
                    <span class="mono">(&exit.name)</span>
                    @if exit.node_id.is_none() && editable {
                        (" ")<span class="tile-detail">(t("no longer carries this node's projects, so \
                             nothing leaves through it"))</span>
                    }
                </p>
            }
        </section>

        @if !page.refused.is_empty() {
            <section class="card stack" id="refused">
                <p class="card-label">(t("Names it was refused"))</p>
                <p class="tile-detail">(t("Asked for by its containers on this node and not on the \
                     list, so they were told there is no such name."))</p>
                <table>
                    <tbody>
                        @for refused in page.refused {
                            <tr>
                                <td class="mono">(&refused.name)</td>
                                <td class="tile-detail">(refused.times)(" × ")</td>
                                <td class="tile-detail">(layout::when(refused.last_at))</td>
                            </tr>
                        }
                    </tbody>
                </table>
            </section>
        }
    }
}
