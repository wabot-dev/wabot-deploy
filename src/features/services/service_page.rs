use hypertext::prelude::*;

use crate::domain::entities::database::Database;
use crate::domain::entities::project::Project;
use crate::domain::entities::release::Release;
use crate::domain::entities::replica::Replica;
use crate::domain::entities::service::{DesiredState, Service, EVERY_TAG};
use crate::domain::entities::volume::Volume;
use crate::domain::values::roles::Access;
use crate::features::databases::database_cards_component;
use crate::features::services::service_push_component::Push;
use crate::features::services::service_running_component::Running;
use crate::features::services::service_state_component::StateCell;
use crate::features::services::texts::t;
use crate::features::services::{
    service_placement_component, service_ports_component, service_push_component,
    service_running_component, service_state_component,
};
use crate::infra::backup::Window;
use crate::infra::network::Node;
use crate::shell::layouts::layout;

/// The seven things that only exist together.
///
/// A managed database's card stands in for the image field and the
/// environment editor, neither of which it has — so this is `Some`
/// exactly when the service is one, and the page never has to ask twice.
pub struct Engine<'a> {
    pub row: &'a Database,
    pub address: Option<String>,
    /// Every name it answers to, from `deploy::certificate_names` — the
    /// same list the certificate is built from, so the page cannot offer
    /// a string naming something the certificate does not cover.
    pub names: &'a [String],
    /// Whether this node is the one signing.
    pub signs_here: bool,
    pub host_port: Option<u16>,
    pub window: &'a Window,
    /// Where this node sends its backups, when that is not this machine.
    pub elsewhere: Option<&'a str>,
    /// A backup on **this** node actually holds it. A form that can only
    /// refuse is worse than the sentence saying why, and what the window
    /// measures is the archive, which can be there with no base copy to
    /// replay onto.
    pub restorable: bool,
}

pub struct ServiceDetail<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    pub allowed: Access,
    pub error: Option<&'a str>,

    pub logs_path: &'a str,
    /// The image tab, because every use of this link is about the image
    /// or the tag: a bare `/settings` is a redirect, and sending somebody
    /// through one to reach a named section is a hop that exists only
    /// because the caller did not say what it wanted.
    pub settings_path: &'a str,

    /// Built once and shared by the badge and the control, so the page
    /// cannot show "running" beside a play button.
    pub cell: &'a StateCell,
    pub observed_running: bool,
    /// The copy on this machine, which is what carries the address and
    /// the reason.
    pub here: Option<&'a Replica>,
    pub exposure: &'a service_ports_component::Exposure,

    pub placements: &'a [Replica],
    pub nodes: &'a [Node],
    pub queued: &'a [String],
    pub running: Running<'a>,
    pub volumes: &'a [Volume],

    pub engine: Option<Engine<'a>>,

    pub releases: &'a [Release],
    pub push: Option<&'a Push>,
}

impl ServiceDetail<'_> {
    fn deploy_path(&self, release: &Release) -> String {
        format!(
            "/projects/{}/services/{}/releases/{}/deploy",
            self.project.slug, self.service.slug, release.id
        )
    }
}

pub fn render<'a>(page: &'a ServiceDetail<'a>) -> impl Renderable + 'a {
    let (project, service) = (page.project, page.service);
    let path = format!("/projects/{}/services/{}", project.slug, service.slug);
    rsx! {
        (layout::style_tag())
        <div class="split">
            <div class="stack-sm">
                <h1>(&service.name)</h1>
            </div>
            <div class="row">
                // Beside Settings, because "what is it saying" is read as
                // often as "how is it configured" — and until now the only
                // answer to the first was the one line kept on the row
                // after it died.
                <a class="btn btn-secondary" href=(page.logs_path)>(t("Logs"))</a>
                @if page.allowed.may_deploy() {
                    <a class="btn btn-secondary" href=(page.settings_path)>(t("Settings"))</a>
                }
                // No "Back to project" here: the breadcrumb's arrow above
                // already goes there, and one thing must not be doable two
                // ways — a second control for it reads as a second
                // destination. Reported by Jorge.
            </div>
        </div>

        @if let Some(message) = page.error {
            (layout::error_note(message))
        }

        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("Container"))</p>
                // `data-state` and `data-address` are what the project's
                // island writes into, keyed by service id — so this page
                // joins that stream instead of opening one of its own.
                // The badge said "Deploying" until somebody reloaded, on
                // the one page they were watching to find out when it
                // stopped.
                <div class="row" data-state=(&service.id)>
                    (service_state_component::state_badge(page.cell))
                    // How many copies there are, beside the word for what
                    // they are doing — where a container's address used to
                    // be, under the label "Address". An address belongs to
                    // one copy, which the replicas table below names one by
                    // one.
                    <span class="tile-detail" data-address=(&service.id)>
                        (page.cell.whereabouts.say())
                    </span>
                    // The page showed the state and withheld the control,
                    // so the one place you go to find out a service is
                    // down was the one place you could not start it.
                    @if page.allowed.may_deploy() && service.is_ours() {
                        (service_state_component::deploy_controls(
                            &project.slug, &service.slug, page.cell, &path))
                    }
                </div>
            </div>
            <dl class="kv">
                <dt>(t("Image"))</dt>
                <dd>(&service.image)</dd>
            </dl>
            // The string somebody came for. A page that shows what a
            // service is should hand it over rather than leave it to be
            // assembled from a hostname in one row and a port in another
            // — which is what a database's card does, and what Jorge
            // asked this one to do.
            //
            // Not for a managed database, and the reason is that this
            // block would lie: it turns a port with a hostname into
            // `https://<name>`, and a database answers the Postgres
            // protocol on a TCP socket. What it offers is its own three
            // cards — the name, the certificate and the published port —
            // each of which says a true thing this one cannot.
            @if !service.kind.is_managed() {
                (service_ports_component::exposure_card(page.exposure, page.observed_running))
            }
            // Always rendered, empty when there is nothing, and hooked to
            // the copy here by its replica id — the stream already carries
            // every replica's reason, so this reads the same value the
            // table below does rather than a second copy of it.
            //
            // It was conditional, so a deployment that failed after the
            // page loaded left this card silent until a refresh while the
            // badge said `Failed`. Reported by Jorge. `:empty` hides it.
            @if let Some(replica) = page.here {
                <p class="failure detail-line"
                   data-replica-detail=(&replica.id)>(
                    replica.last_error.clone().unwrap_or_default()
                )</p>
            }
        </section>

        @if let Some(engine) = &page.engine {
            (database_cards_component::database_card(
                engine.row, page.placements, engine.address.clone(),
                service.memory_limit, engine.names, engine.signs_here, engine.host_port))
            // How far back it can be taken. Beside the database card
            // because it is a fact about this database rather than about
            // the node — even though what decides it is a switch on the
            // node's page, which is why the card says so when it is off.
            (database_cards_component::window_card(engine.window, engine.elsewhere))
            @if engine.restorable {
                (database_cards_component::restore_card(
                    &project.slug, &service.slug, engine.window,
                    &format!("{}-restored", service.slug), engine.elsewhere))
            }
        }

        @if service.is_ours() {
            (service_running_component::running_card(
                page.placements, page.nodes, page.queued,
                service.desired_state == DesiredState::Stopped, &page.running))
            // Only where there is one to show. A service that declares no
            // volume keeps nothing, and a card saying so would be a row of
            // dashes explaining an absence nobody asked about.
            @if !page.volumes.is_empty() {
                (service_running_component::disks_card(
                    page.volumes, page.placements, page.nodes, &page.running))
            }
        } @else {
            (service_placement_component::from_elsewhere_card(
                project,
                service,
                &page.placements
                    .iter()
                    .filter(|replica| replica.is_here())
                    .cloned()
                    .collect::<Vec<_>>(),
                page.placements.iter().all(|replica| replica.evicted()),
            ))
        }

        // A managed database has no releases and no tag to watch: its
        // image is written by the node, pinned to the major version the
        // operator chose, and a push to this node's registry has nothing
        // to do with it. The card said "nothing has been pushed yet —
        // create a push token and push an image", which is an instruction
        // that would achieve nothing here.
        @if !service.kind.is_managed() {
            <section class="stack">
                <div class="split">
                    <p class="card-label">(t("Releases"))</p>
                    // **Only where a push here could match.** The badge
                    // appeared for every service, including one running an
                    // image from another registry — and for those it read
                    // the *foreign* tag, so a service on
                    // `docker.io/library/nginx:alpine` announced
                    // `watching :alpine` directly above this card's own
                    // sentence saying a push to this node can never land
                    // on it. Three answers to one question. Reported by
                    // Jorge.
                    //
                    // The tag itself defaults to `latest` — see
                    // `Service::tracked_tag`. A service pinned to
                    // `…/app:v2` in this registry has to say so in that
                    // field, and this badge is what makes the difference
                    // visible instead of silent.
                    @if let Some(Push::To(_) | Push::Elsewhere(_)) = page.push {
                        @let watched = service.tracked_tag().unwrap_or_else(|| "latest".into());
                        // `watching :*` is a badge nobody can read. The
                        // star is the way it is *written*, not the way it
                        // is said.
                        @if watched == EVERY_TAG {
                            <span class="who">(t("watching every tag"))</span>
                        } @else {
                            <span class="who">(format!("watching :{watched}"))</span>
                        }
                    }
                </div>
                @if page.releases.is_empty() {
                    <p class="tile-detail">(t("Nothing has been pushed yet. Create a push token on the \
                         project page and push an image to this repository."))</p>
                } @else {
                    <table>
                        <thead>
                            <tr>
                                <th>(t("Image"))</th>
                                <th>(t("From"))</th>
                                <th>(t("State"))</th>
                                <th></th>
                            </tr>
                        </thead>
                        <tbody>
                            @for release in page.releases {
                                <tr>
                                    // **The tag first, the digest under
                                    // it.** This showed only the short
                                    // digest under a column headed
                                    // "Image", and eight hex characters
                                    // are not something anybody can tell
                                    // apart — least of all "which of these
                                    // did I push". The reference has
                                    // always been on the row and nothing
                                    // rendered it. Reported by Jorge.
                                    //
                                    // The tag rather than the whole
                                    // reference, because the repository is
                                    // identical on every row and a column
                                    // of the same forty characters
                                    // distinguishes nothing. The whole
                                    // thing is in the title, with the
                                    // digest.
                                    <td class="mono"
                                        title=(format!("{}\n{}", release.reference, release.digest))>
                                        (release.tag())
                                        // A block, because `span` put the
                                        // digest *beside* the tag with
                                        // nothing between them —
                                        // `newsha256:af618a130cbe`, one
                                        // unreadable word where the
                                        // comment above promised two
                                        // lines. `.tile-detail` is written
                                        // for a block and says nothing
                                        // about `display`, so putting it
                                        // on an inline element styled the
                                        // colour and left the layout to
                                        // the default. Reported by Jorge.
                                        <div class="tile-detail">(release.short_digest())</div>
                                    </td>
                                    <td class="tile-detail">(release.source.label())</td>
                                    <td>
                                        @if release.deployed_at.is_some() {
                                            <span class="badge badge-success">
                                                <span class="dot dot-success"></span>
                                                (t("Running"))
                                            </span>
                                        }
                                    </td>
                                    <td>
                                        @if page.allowed.may_deploy() && release.deployed_at.is_none() {
                                            <form method="post" action=(page.deploy_path(release))>
                                                <button class="btn btn-secondary btn-sm"
                                                        type="submit">(t("Deploy this"))</button>
                                            </form>
                                        }
                                    </td>
                                </tr>
                            }
                        </tbody>
                    </table>
                }

                // How to put one here. The page said "push an image to
                // this repository" and left somebody to work out the three
                // parts of the reference — the node's host, the name under
                // the project, and the one tag this service watches — from
                // three other pages.
                @if let Some(push) = page.push {
                    (service_push_component::push_example(push, service.auto_deploy, page.settings_path))
                }
            </section>
        }
    }
}
