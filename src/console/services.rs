//! Services: the create form, and the POST behind it.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use crate::deploy::dns;
use crate::platform::{access, config_history, ports, releases, services};

use super::auth::{back_with_error, field, read_form, see_other, signed_in, SessionMiddleware};
use super::shell::{Area, Frame};
use super::{layout, ConsoleState};

#[derive(Debug, Deserialize, Validate)]
pub struct ServiceForm {
    pub project: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ServicePage {
    pub project: String,
    pub service: String,
    pub error: Option<String>,
}

/// The same service, from the page that changes it.
///
/// Its own struct rather than a shared one: `checked` is produced by
/// the port form and belongs where that form is, and a page that
/// accepts a parameter nothing sends it invites somebody to send it.
#[derive(Debug, Deserialize, Validate)]
pub struct ServiceSettings {
    pub project: String,
    pub service: String,
    pub error: Option<String>,
    /// What a DNS check just said, carried back from the POST that ran
    /// it. Not stored: it describes the world a second ago, and the
    /// next attempt asks again.
    pub checked: Option<String>,
}

#[injectable]
pub struct ServicePages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl ServicePages {
    #[view("/projects/:project/services/new")]
    #[middleware(SessionMiddleware)]
    async fn new_service(&self, query: ServiceForm) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        if !allowed.may_deploy() {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        }

        let action = format!("/projects/{}/services", project.slug);
        let back = format!("/projects/{}", project.slug);
        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("/projects/{}/services/new", project.slug),
        )
        .allowing(allowed);

        layout::head("Create service");
        let body = rsx! {
            (layout::style_tag())
                <div class="stack-sm">
                    <h1>("Create service")</h1>
                    <p class="slug-preview">(&project.slug)</p>
                </div>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }
                <form method="post" action=(&action) class="card stack">
                    <label for="name">("Name")</label>
                    <input id="name" name="name" type="text" required autofocus>

                    <label for="image">("Image")</label>
                    <input id="image" name="image" type="text" class="mono"
                           placeholder="docker.io/library/nginx:alpine" required>
                    <p class="field-hint">(
                        "A reference containerd can resolve. Fully qualified — \
                         there is no implicit registry here."
                    )</p>

                    <label for="env">("Environment")</label>
                    <textarea id="env" name="env" rows="6" class="mono"
                              placeholder="KEY=value"></textarea>
                    <p class="field-hint">(
                        "One KEY=value per line. Everything after the first = is \
                         the value, so a value may contain one."
                    )</p>

                    <div class="actions">
                        <button type="submit">("Create service")</button>
                        <a class="btn btn-ghost" href=(&back)>("Cancel")</a>
                    </div>
                </form>
        }
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }

    /// What this service is doing, and where to reach it.
    ///
    /// Reading, and the one action that belongs with it — deploying a
    /// release. Everything that *configures* the service lives on
    /// [`Self::settings`]: the two answer different questions, and a
    /// page somebody keeps open while a deployment lands should not be
    /// a page covered in forms they are not filling in.
    #[view("/projects/:project/services/:service")]
    #[middleware(SessionMiddleware)]
    async fn service(&self, query: ServicePage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, &query.service).await?
        else {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        };

        let ports = ports::of_service(&self.state.database, &service.id).await?;
        let releases = releases::of_service(&self.state.database, &service.id).await?;

        // Whether each HTTPS name has a certificate yet. Asked here
        // rather than assumed: the certificate arrives seconds after
        // the route does, and a page that showed the name as ready
        // would be wrong for exactly the window somebody is looking at
        // it.
        let mut secured = std::collections::BTreeSet::new();
        for port in &ports {
            if let Some(hostname) = &port.hostname {
                if crate::edge::certs::load(&self.state.database, hostname)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|certificate| certificate.issuer != "self-signed")
                {
                    secured.insert(hostname.clone());
                }
            }
        }
        let observed = self
            .state
            .deployer
            .observe_service(&project, &service)
            .await;
        // The address and the reason live on the copy that runs here —
        // a service is *n* of them now, and this page still shows one.
        let here = crate::platform::replicas::here_for(&self.state.database, &service.id).await?;
        // Where every copy of it runs, and the nodes it could run on.
        // Both only matter when this node is the one that decides — a
        // service that arrived from elsewhere is placed from there.
        let placements =
            crate::platform::replicas::of_service(&self.state.database, &service.id).await?;
        let nodes = crate::network::all(&self.state.database).await?;
        let deploying = crate::deploy::jobs::deploying(&self.state.container)
            .await
            .contains(&service.id);
        // Taken before the markup moves it: the island below names the
        // stream this page joins, and `rsx!` captures by move.
        let project_slug = project.slug.clone();
        // Built once and shared by the badge and the control, so the
        // page cannot show "running" beside a play button.
        let cell = super::projects::state_cell(
            &observed,
            deploying,
            here.as_ref().and_then(|replica| replica.address.as_deref()),
        );
        let back = format!("/projects/{}", project.slug);
        let settings = format!(
            "/projects/{}/services/{}/settings",
            project.slug, service.slug
        );
        let domain = crate::node::settings::domain(&self.state.database, &self.state.config).await;

        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("/projects/{}/services/{}", project.slug, service.slug),
        )
        .allowing(allowed);

        layout::head(&service.name);
        let body = rsx! {
            (layout::style_tag())
                <div class="split">
                    <div class="stack-sm">
                        <h1>(&service.name)</h1>
                        <p class="slug-preview">(&project.slug)(" / ")(&service.slug)</p>
                    </div>
                    <div class="row">
                        @if allowed.may_deploy() {
                            <a class="btn btn-secondary" href=(&settings)>("Settings")</a>
                        }
                        <a class="btn btn-ghost" href=(&back)>("Back to project")</a>
                    </div>
                </div>

                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }

                <section class="card stack">
                    <div class="split">
                        <p class="card-label">("Container")</p>
                        // `data-state` and `data-address` are what the
                        // project's island writes into, keyed by service
                        // id — so this page joins that stream instead of
                        // opening one of its own. The badge said
                        // "Deploying" until somebody reloaded, on the
                        // one page they were watching to find out when
                        // it stopped.
                        <div class="row" data-state=(&service.id)>
                            (super::projects::state_badge(&cell))
                            // The page showed the state and withheld the
                            // control, so the one place you go to find
                            // out a service is down was the one place
                            // you could not start it.
                            @if allowed.may_deploy() && service.is_ours() {
                                (super::projects::deploy_controls(
                                    &project.slug,
                                    &service.slug,
                                    &cell,
                                    &format!(
                                        "/projects/{}/services/{}",
                                        project.slug, service.slug
                                    ),
                                ))
                            }
                        </div>
                    </div>
                    <dl class="kv">
                        <dt>("Image")</dt>
                        <dd>(&service.image)</dd>
                        <dt>("Address")</dt>
                        <dd class="mono" data-address=(&service.id)>(
                            here.as_ref()
                                .and_then(|r| r.address.clone())
                                .unwrap_or_else(|| "not running".into())
                        )</dd>
                    </dl>
                    @if let Some(failure) = here.as_ref().and_then(|r| r.last_error.as_ref()) {
                        <p class="failure">(failure)</p>
                    }
                </section>

                @if service.is_ours() {
                    (placement_card(&project, &service, &placements, &nodes))
                } @else {
                    (from_elsewhere_card(
                        &project,
                        &service,
                        &placements
                            .iter()
                            .filter(|replica| replica.is_here())
                            .cloned()
                            .collect::<Vec<_>>(),
                        placements.iter().all(|replica| replica.evicted()),
                    ))
                }

                <section class="stack">
                    <div class="split">
                        <p class="card-label">("Releases")</p>
                        <span class="who">(
                            format!("watching :{}", crate::platform::images::tracked_tag(
                                &service.image, service.track_tag.as_deref()
                            ).unwrap_or_else(|| "—".into()))
                        )</span>
                    </div>
                    @if releases.is_empty() {
                        <p class="tile-detail">(
                            "Nothing has been pushed yet. Create a push token on the \
                             project page and push an image to this repository."
                        )</p>
                    } @else {
                        <table>
                            <thead>
                                <tr>
                                    <th>("Image")</th>
                                    <th>("From")</th>
                                    <th>("State")</th>
                                    <th></th>
                                </tr>
                            </thead>
                            <tbody>
                                @for release in &releases {
                                    <tr>
                                        <td class="mono" title=(&release.digest)>
                                            (release.short_digest())
                                        </td>
                                        <td class="tile-detail">(release.source.label())</td>
                                        <td>
                                            @if release.deployed_at.is_some() {
                                                <span class="badge badge-success">
                                                    <span class="dot dot-success"></span>
                                                    ("Running")
                                                </span>
                                            }
                                        </td>
                                        <td>
                                            @if allowed.may_deploy()
                                                && release.deployed_at.is_none() {
                                                <form method="post"
                                                      action=(format!(
                                                          "/projects/{}/services/{}/releases/{}/deploy",
                                                          project.slug, service.slug, release.id
                                                      ))>
                                                    <button class="btn btn-secondary btn-sm"
                                                            type="submit">("Deploy this")</button>
                                                </form>
                                            }
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    }
                </section>

                <section class="stack">
                    <p class="card-label">("Reachable at")</p>
                    @if ports.is_empty() {
                        <p class="tile-detail">(
                            "This service exposes nothing. That is the right answer \
                             for a worker; a port is added in settings."
                        )</p>
                    } @else {
                        <table>
                            <thead>
                                <tr>
                                    <th>("Container")</th>
                                    <th>("Reachable at")</th>
                                </tr>
                            </thead>
                            <tbody>
                                @for port in &ports {
                                    <tr>
                                        <td class="mono">(port.container_port)</td>
                                        <td class="mono reach">
                                            <span>(reachable_at(port, domain.as_deref()))</span>
                                            @if port.hostname.as_ref()
                                                .is_some_and(|host| !secured.contains(host)) {
                                                <span class="badge badge-info">
                                                    <span class="dot dot-info dot-pulse"></span>
                                                    ("Certificate on the way")
                                                </span>
                                            }
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    }
                </section>
        }
        .render()
        .into_inner();

        // The project's own island, on the project's own stream. It
        // writes by service id into `data-state` and `data-address`,
        // and this page has one of each — so a page showing one service
        // costs no second endpoint and cannot drift from the list.
        //
        // Rendered first, then wrapped: `rsx!` expands to a closure
        // that captures by move, and nesting one inside the island's
        // would have both wanting the same borrows.
        let body = wabot::ui::hypertext::island(
            "project-live",
            &serde_json::json!({ "project": project_slug }),
            hypertext::Raw::dangerously_create(&body),
        )
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }

    /// How this service is configured, and everything that changes it.
    ///
    /// Split from [`Self::service`] because the two are read at
    /// different moments: the service page is open while a deployment
    /// lands, this one is opened to change something and left again.
    /// Every form that used to crowd that page is here, and every POST
    /// behind them comes back here rather than to the page it left.
    #[view("/projects/:project/services/:service/settings")]
    #[middleware(SessionMiddleware)]
    async fn settings(&self, query: ServiceSettings) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, &query.service).await?
        else {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        };

        let here = format!("/projects/{}/services/{}", project.slug, service.slug);
        // Every control on this page is one a viewer would be refused,
        // so they get the page they may read instead of a form that
        // lies to them. The check that matters is still on each POST.
        if !allowed.may_deploy() {
            return Ok(Redirect::found(here).into());
        }

        let ports = ports::of_service(&self.state.database, &service.id).await?;
        let history = config_history::of_service(&self.state.database, &service.id).await?;
        let env_text: String = service
            .env
            .iter()
            .map(|(key, value)| format!("{key}={value}\n"))
            .collect();

        let mut secured = std::collections::BTreeSet::new();
        for port in &ports {
            if let Some(hostname) = &port.hostname {
                if crate::edge::certs::load(&self.state.database, hostname)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|certificate| certificate.issuer != "self-signed")
                {
                    secured.insert(hostname.clone());
                }
            }
        }

        // One answer per name the service is served under. The node's
        // domain is not a special case of this — it is the same
        // question asked about a different name.
        let mut certificates = Vec::new();
        for port in &ports {
            let Some(hostname) = &port.hostname else {
                continue;
            };
            let facts = super::certificate_facts_for(&self.state, hostname).await;
            let state = super::nodes::CertificateState::read(
                &facts,
                Some(hostname),
                // Not the node's stored failure: it records one reason
                // for the whole node, and attributing it to this name
                // would be the console guessing.
                None,
                self.state.certificates.phase(),
                self.state.config.acme.disabled,
            );
            let policy =
                crate::edge::policy::for_name(&self.state.database, &self.state.config, hostname)
                    .await;
            let cells = super::nodes::certificate_cells(&state, &facts, Some(hostname));
            certificates.push((port.id.clone(), hostname.clone(), cells, policy));
        }

        let add = format!("{here}/ports");
        let domain = crate::node::settings::domain(&self.state.database, &self.state.config).await;
        let suggestion = match &domain {
            Some(domain) => {
                let name = dns::suggested_hostname(&service.slug, &project.slug, domain);
                let wildcard = dns::wildcard_works(domain).await;
                Some((name, wildcard))
            }
            None => None,
        };

        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("{here}/settings"),
        )
        .allowing(allowed);

        layout::head(&format!("{} settings", service.name));
        let body = rsx! {
            (layout::style_tag())
                <div class="split">
                    <div class="stack-sm">
                        <h1>("Settings")</h1>
                        <p class="slug-preview">(&project.slug)(" / ")(&service.slug)</p>
                    </div>
                    <a class="btn btn-ghost" href=(&here)>("Back to service")</a>
                </div>

                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }
                @if let Some(message) = &query.checked {
                    <p class="note">(message)</p>
                }

                <section class="stack">
                    <p class="card-label">("Releases")</p>
                    <form method="post" action=(format!("{here}/tracking")) class="card stack">
                        <label for="track_tag">("Tag to watch")</label>
                        <input id="track_tag" name="track_tag" type="text" class="mono"
                               value=(service.track_tag.clone().unwrap_or_default())
                               placeholder="latest">
                        <label class="check">
                            // `checked="false"` checks it. This box read
                            // as on whatever the row said, and saving the
                            // form then turned it on for real.
                            @if service.auto_deploy {
                                <input type="checkbox" name="auto_deploy" value="1" checked>
                            } @else {
                                <input type="checkbox" name="auto_deploy" value="1">
                            }
                            ("Deploy a push automatically")
                        </label>
                        <p class="field-hint">(
                            "Without this, a push is recorded as a release and waits \
                             for somebody to deploy it from the service page."
                        )</p>
                        <div class="actions">
                            <button type="submit">("Save")</button>
                        </div>
                    </form>
                </section>

                <section class="stack">
                    <p class="card-label">("Environment")</p>
                    <form method="post" action=(format!("{here}/env")) class="card stack">
                        <textarea name="env" rows="6" class="mono"
                                  placeholder="KEY=value">(&env_text)</textarea>
                        <p class="field-hint">(
                            "Saving redeploys the service with these values. The image \
                             it runs does not change."
                        )</p>
                        <div class="actions">
                            <button type="submit">("Save environment")</button>
                        </div>
                    </form>

                    @if !history.is_empty() {
                        <table>
                            <tbody>
                                @for revision in history.iter().skip(1) {
                                    <tr>
                                        <td class="tile-detail">
                                            (revision.env.len())(" values · ")(&revision.reason)
                                        </td>
                                        <td>
                                            <form method="post"
                                                  action=(format!(
                                                      "{here}/env/{}/revert", revision.id
                                                  ))>
                                                <button class="btn btn-ghost btn-sm"
                                                        type="submit">("Restore")</button>
                                            </form>
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    }
                </section>

                <section class="stack">
                    <p class="card-label">("Ports")</p>
                    @if ports.is_empty() {
                        <p class="tile-detail">(
                            "This service exposes nothing. That is the right answer for \
                             a worker; add a port for anything that listens."
                        )</p>
                    } @else {
                        <table>
                            <thead>
                                <tr>
                                    <th>("Container")</th>
                                    <th>("Reachable at")</th>
                                    <th></th>
                                </tr>
                            </thead>
                            <tbody>
                                @for port in &ports {
                                    <tr>
                                        <td class="mono">(port.container_port)</td>
                                        <td class="mono reach">
                                            <span>(reachable_at(port, domain.as_deref()))</span>
                                            @if port.hostname.as_ref()
                                                .is_some_and(|host| !secured.contains(host)) {
                                                <span class="badge badge-info">
                                                    <span class="dot dot-info dot-pulse"></span>
                                                    ("Certificate on the way")
                                                </span>
                                            }
                                        </td>
                                        <td>
                                            <form method="post"
                                                  action=(format!("{add}/{}/delete", port.id))>
                                                <button class="btn btn-ghost destructive btn-sm"
                                                        type="submit">
                                                    ("Remove")
                                                </button>
                                            </form>
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    }

                    (port_form(&add, &suggestion, account.is_admin()))
                </section>

                <section class="card stack">
                    <p class="card-label">("Danger zone")</p>
                    <p class="tile-detail">(
                        "Deleting a service stops its container and removes it. The \
                         images it was built from stay in the registry."
                    )</p>
                    <form method="post" action=(format!("{here}/delete"))>
                        <button class="btn btn-danger" type="submit">("Delete service")</button>
                    </form>
                </section>

                @if !certificates.is_empty() {
                    <section class="stack">
                        <p class="card-label">("Certificates")</p>
                        <p class="tile-detail">(
                            "One per hostname. A node with no public DNS, or a name a \
                             certificate authority cannot reach, is what the other two \
                             answers are for."
                        )</p>
                        @for (port_id, hostname, cells, policy) in &certificates {
                            <div class="card stack">
                                <div class="split">
                                    <p class="mono">(hostname)</p>
                                    <span class=(cells.badge)>
                                        <span class=(cells.dot)></span>
                                        <span>(cells.word)</span>
                                    </span>
                                </div>
                                <dl class="kv">
                                    <dt>("Issuer")</dt>
                                    <dd>(&cells.issuer)</dd>
                                    <dt>("Renews in")</dt>
                                    <dd>(&cells.renews)</dd>
                                </dl>
                                (super::nodes::certificate_source_form(
                                    &format!("{here}/ports/{port_id}/certificate"),
                                    policy,
                                ))
                            </div>
                        }
                    </section>
                }
        }
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }
}

/// Declaring a port, as an island host.
///
/// Its own function for the reason `Frame::render` gives: an `rsx!`
/// expands to a closure that captures by move, so nesting one inside
/// another makes both want the same `add` and `suggestion`. Rendering
/// this one first ends its borrow before the page's begins.
fn port_form<'a>(
    add: &'a str,
    suggestion: &'a Option<(String, bool)>,
    admin: bool,
) -> impl Renderable + 'a {
    wabot::ui::hypertext::island_bare(
        "fields",
        rsx! {
                        <form method="post" action=(add) class="card stack">
                            <label for="container_port">("Container port")</label>
                            <input id="container_port" name="container_port" type="number"
                                   min="1" max="65535" placeholder="80" required>
                            <p class="field-hint">(
                                "What the process listens on inside the container."
                            )</p>

                            <label class="check">
                                <input type="checkbox" name="publish" value="1">
                                ("Publish on the node's public address (raw TCP)")
                            </label>
                            <p class="field-hint">(
                                "For a database or anything that is not HTTP. The node \
                                 picks the outside port. It is reachable from the whole \
                                 internet unless a firewall says otherwise."
                            )</p>

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
                                        ("Serve over HTTPS at a hostname")
                                    </label>
                                    <div class="stack" data-when="https">
                                        <p class="field-hint">(
                                            "A wildcard record covers this node, so this \
                                             name already resolves here. Leave it as it is."
                                        )</p>
                                        <input name="hostname" type="text" class="mono"
                                               value=(name) data-required-when="https">
                                    </div>
                                }
                                Some((name, false)) => {
                                    <label class="check">
                                        <input type="checkbox" name="https" value="1">
                                        ("Serve over HTTPS at a hostname")
                                    </label>
                                    <div class="stack" data-when="https">
                                        <p class="field-hint">(
                                            "No wildcard record answers for this node, so "
                                        )(name)(
                                            " will not resolve. Either add \
                                             *.<node domain> pointing at this node, or type \
                                             a hostname you have already pointed here — it \
                                             is checked before it is accepted."
                                        )</p>
                                        <input name="hostname" type="text" class="mono"
                                               placeholder="api.example.com"
                                               data-required-when="https">
                                    </div>
                                }
                                None => {
                                    <label class="check">
                                        <input type="checkbox" name="https" value="1" disabled>
                                        ("Serve over HTTPS at a hostname")
                                    </label>
                                    <p class="field-hint">(
                                        "This node has no domain of its own, so it cannot \
                                         check that a name points here — and it will not \
                                         route one it could not check. The node needs a \
                                         domain before anything can be served over HTTPS."
                                    )</p>
                                    @if admin {
                                        <a class="btn btn-secondary btn-sm" href="/nodes">(
                                            "Set the node's domain"
                                        )</a>
                                    } @else {
                                        <p class="field-hint">(
                                            "Ask whoever runs this node to set one."
                                        )</p>
                                    }
                                }
                            }

                            <div class="actions">
                                <button type="submit">("Add port")</button>
                            </div>
                        </form>
        },
    )
}

/// Where a port can be reached from, in one cell.
fn reachable_at(port: &crate::platform::ports::Port, node_domain: Option<&str>) -> String {
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

/// Where to send the browser after a control was pressed.
///
/// The form says, because only the form knows: the same play and stop
/// are on the project's list and on the service's own page, and sending
/// somebody back to the list from the page they were reading is the
/// console deciding they meant to leave.
///
/// A path this console put in its own markup, never a `Referer` — that
/// would let a page elsewhere choose where a submit lands. Anything
/// that is not a path falls back to where it always went.
async fn returning_to(request: Request, fallback: &str) -> String {
    let form = match read_form(request).await {
        Ok(form) => form,
        Err(_) => return fallback.to_string(),
    };
    let from = field(&form, "from");
    match from.starts_with('/') && !from.starts_with("//") {
        true => from.to_string(),
        false => fallback.to_string(),
    }
}

/// Where every copy of this service runs, and how to change it.
///
/// One row per replica, each naming the node it is on. The first is
/// this node by default and there is nothing special about it — "stays
/// here" is a placement like any other, which is the whole point of
/// administering a service from where it was created rather than from
/// the machine it happens to be on.
fn placement_card<'a>(
    project: &'a crate::platform::projects::Project,
    service: &'a services::Service,
    placements: &'a [crate::platform::replicas::Replica],
    nodes: &'a [crate::network::Node],
) -> impl Renderable + 'a {
    let action = format!(
        "/projects/{}/services/{}/placement",
        project.slug, service.slug
    );
    rsx! {
        <section class="stack">
            <div class="split">
                <p class="card-label">("Where this runs")</p>
                <span class="who">(format!("{} replica(s)", placements.len()))</span>
            </div>

            <form method="post" action=(&action) class="card stack">
                <table>
                    <thead>
                        <tr><th>("Replica")</th><th>("Node")</th><th>("State")</th></tr>
                    </thead>
                    <tbody>
                        @for replica in placements {
                            <tr>
                                <td class="mono">("#")(replica.slot)</td>
                                <td>
                                    <select name=(format!("slot-{}", replica.slot))>
                                        @for node in nodes {
                                            (node_option(node, replica))
                                        }
                                    </select>
                                </td>
                                <td>(placement_state(replica))</td>
                            </tr>
                        }
                    </tbody>
                </table>

                <label for="replicas">("How many")</label>
                <input id="replicas" name="replicas" type="number" min="1" max="16"
                       value=(placements.len().to_string())>
                <p class="field-hint">(
                    "Adding one puts it here; move it afterwards. Removing takes the \
                     highest-numbered ones away, and a replica on another node cannot \
                     be removed from here yet — nothing can tell that node to stop, so \
                     removing the row would leave its container running with nothing \
                     naming it."
                )</p>
                <div class="actions">
                    <button type="submit">("Save placement")</button>
                </div>
            </form>
        </section>
    }
}

/// One node in a replica's selector.
///
/// This node is the **empty** value, because "here" is the absence of a
/// placement elsewhere — the same shape the row has, so the form and
/// the column agree without anything translating between them.
fn node_option<'a>(
    node: &'a crate::network::Node,
    replica: &'a crate::platform::replicas::Replica,
) -> impl Renderable + 'a {
    let value = match node.is_self {
        true => String::new(),
        false => node.id.clone(),
    };
    let chosen = match node.is_self {
        true => replica.is_here(),
        false => replica.node_id.as_deref() == Some(node.id.as_str()),
    };
    rsx! {
        @if chosen {
            <option value=(value) selected>(&node.name)</option>
        } @else {
            <option value=(value)>(&node.name)</option>
        }
    }
}

/// What a replica is doing, in the words its own node reported.
fn placement_state(replica: &crate::platform::replicas::Replica) -> impl Renderable + '_ {
    rsx! {
        @if replica.evicted() {
            <span class="badge badge-warning">("Evicted there")</span>
        } @else if let Some(failure) = &replica.last_error {
            <span class="badge badge-danger">
                <span class="dot dot-danger"></span>("Failed")
            </span>
            <p class="failure">(failure)</p>
        } @else if let Some(address) = &replica.address {
            <span class="badge badge-success">
                <span class="dot dot-success"></span>("Running")
            </span>
            <span class="tile-detail">(" ")(address)</span>
        } @else if replica.is_here() {
            <span class="badge">("Not running")</span>
        } @else {
            // Placed elsewhere and nothing has come back about it. The
            // node reports when it collects, so this is the honest word
            // until it does — not "failed", which would be this page
            // inventing an outcome nobody reported.
            <span class="badge badge-info">
                <span class="dot dot-info dot-pulse"></span>("Waiting for that node")
            </span>
        }
    }
}

/// A service this node did not create.
///
/// Named rather than hidden: somebody looking at it needs to know which
/// node to go and argue with, and a page that simply showed fewer
/// buttons would leave them wondering what they had done wrong.
fn from_elsewhere_card<'a>(
    project: &'a crate::platform::projects::Project,
    service: &'a services::Service,
    mine: &'a [crate::platform::replicas::Replica],
    evicted: bool,
) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">("From another node")</p>
            <p>(
                "This service is administered from the node that placed it here, and \
                 nothing on this page will change it."
            )</p>
            <dl class="kv">
                <dt>("Placed by")</dt>
                <dd class="mono">(service.origin_node_id.as_deref().unwrap_or("—"))</dd>
            </dl>

            // What is actually on *this* machine. The page used to name
            // the node that sent it and stop there, so the operator
            // could not see how many copies they were running or what
            // any of them was doing — which is the one thing they can
            // see about it that the other node cannot.
            <table>
                <thead>
                    <tr><th>("Running here")</th><th>("State")</th></tr>
                </thead>
                <tbody>
                    @for replica in mine {
                        <tr>
                            <td class="mono">("#")(replica.slot)</td>
                            <td>(placement_state(replica))</td>
                        </tr>
                    }
                </tbody>
            </table>
            <p class="field-hint">(
                "The machine is yours: throwing it out is something you can always do, \
                 and it is the only thing here that is."
            )</p>
        </section>

        <section class="card stack">
            <p class="card-label">("Throw it off this node")</p>
            @if evicted {
                <p>(
                    "Already thrown out. Its containers are stopped and the node that \
                     placed it has been told — or will be, the next time it asks."
                )</p>
                <p class="field-hint">(
                    "The rows stay because they are what carries that news. Removing \
                     them here would leave the other node sending the same instruction \
                     again."
                )</p>
            } @else {
                <p>(
                    "Stops its containers here and tells the node that placed it to \
                     stop asking. It cannot be undone from this side — that node \
                     decides what happens next, and it may place it somewhere else."
                )</p>
                <form method="post" action=(format!(
                    "/projects/{}/services/{}/evict", project.slug, service.slug
                ))>
                    <button class="btn btn-ghost destructive" type="submit">
                        ("Throw it out")
                    </button>
                </form>
            }
        </section>
    }
}

#[injectable]
pub struct ServiceApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl ServiceApi {
    #[post("/projects/:project/services")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };

        // The slug out of the path. Taken from the URI rather than a
        // typed request because `#[raw]` hands over the whole thing —
        // the trade for being able to answer with a 303.
        let path = request.uri().path().to_string();
        let Some(slug) = project_slug(&path) else {
            return Ok(see_other("/?error=no+such+project"));
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, slug).await?
        else {
            return Ok(see_other("/?error=no+such+project"));
        };
        if !allowed.may_deploy() {
            return Ok(back_with_error(
                &format!("/projects/{}", project.slug),
                "you may look at this project, not change it",
            ));
        }
        let form_url = format!("/projects/{}/services/new", project.slug);

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let env = match parse_env(field(&form, "env")) {
            Ok(env) => env,
            Err(message) => return Ok(back_with_error(&form_url, &message)),
        };

        match services::create(
            &self.state.database,
            &project.id,
            field(&form, "name"),
            field(&form, "image"),
            &env,
        )
        .await
        {
            Ok(service) => {
                // Deployed on creation. The alternative — create, then
                // press Deploy — makes the common case two steps and
                // leaves a row that describes nothing.
                //
                // Straight to the service's own page: declaring ports
                // is the next thing anybody does, and it is where the
                // reason lives if this deployment failed.
                self.enqueue(&service.id, None).await;
                Ok(see_other(&format!(
                    "/projects/{}/services/{}",
                    project.slug, service.slug
                )))
            }
            Err(error) => Ok(back_with_error(&form_url, &error.to_string())),
        }
    }

    /// Declare a port on a service.
    ///
    /// A hostname is checked against DNS *before* it is stored. The
    /// alternative — store it, then find out at certificate time — is a
    /// service that looks configured and answers nothing, with the
    /// reason in a log the operator never sees.
    #[post("/projects/:project/services/:service/ports")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add_port(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        // Back to the page the form is on, not the one it used to
        // share with the service's state.
        let here = format!(
            "/projects/{}/services/{}/settings",
            project.slug, service.slug
        );

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let container_port = match parse_port(field(&form, "container_port")) {
            Ok(Some(port)) => port,
            Ok(None) => return Ok(back_with_error(&here, "a container port is required")),
            Err(message) => return Ok(back_with_error(&here, message)),
        };
        let publish = checked(&form, "publish");

        // The hostname is only read when the box is ticked. A name left
        // in the field from a previous attempt must not quietly become
        // a route nobody asked for.
        let hostname = if checked(&form, "https") {
            let typed = field(&form, "hostname");
            if typed.is_empty() {
                return Ok(back_with_error(
                    &here,
                    "serving over HTTPS needs a hostname",
                ));
            }
            let hostname = ports::normalize_hostname(typed);

            let Some(node_domain) =
                crate::node::settings::domain(&self.state.database, &self.state.config).await
            else {
                return Ok(back_with_error(
                    &here,
                    "this node has no domain of its own, so it cannot check whether \
                     that name points here — set one on the node page first",
                ));
            };

            let outcome = dns::resolves_here(&hostname, &node_domain).await;
            if !outcome.ok() {
                return Ok(back_with_error(&here, &outcome.explain(&hostname)));
            }
            Some(hostname)
        } else {
            None
        };

        match ports::create(
            &self.state.database,
            &service.id,
            container_port,
            publish,
            hostname.as_deref(),
        )
        .await
        {
            Ok(port) => {
                // Redeployed, because a port is part of how the
                // container is built: published ports are iptables
                // rules made when it joins the network, not something
                // that can be added to a running one.
                self.enqueue(&service.id, None).await;

                let confirmed = port
                    .hostname
                    .map(|hostname| format!("{hostname} resolves here, and is now routed"));
                Ok(see_other(&match confirmed {
                    Some(message) => format!(
                        "{here}?{}",
                        form_urlencoded::Serializer::new(String::new())
                            .append_pair("checked", &message)
                            .finish()
                    ),
                    None => here,
                }))
            }
            Err(error) => Ok(back_with_error(&here, &error.to_string())),
        }
    }

    /// Remove a port.
    #[post("/projects/:project/services/:service/ports/:port/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete_port(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let segments = super::auth::segments(&path);
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        // Back to the page the form is on, not the one it used to
        // share with the service's state.
        let here = format!(
            "/projects/{}/services/{}/settings",
            project.slug, service.slug
        );

        let ["projects", _, "services", _, "ports", port_id, "delete"] = segments.as_slice() else {
            return Ok(see_other(&here));
        };

        // Only this service's ports, or an id from another page would
        // delete somebody else's route.
        let owned = ports::of_service(&self.state.database, &service.id)
            .await?
            .into_iter()
            .any(|port| port.id == *port_id);

        if owned {
            ports::delete(&self.state.database, port_id).await?;
            self.enqueue(&service.id, None).await;
        }
        Ok(see_other(&here))
    }

    /// Choose where one hostname's certificate comes from.
    ///
    /// The same three answers as the node's own name, because it is
    /// the same question. The file pair is read and checked here,
    /// before the choice is stored: a policy that cannot work would
    /// fail on every pass of the renewal loop with the reason in the
    /// journal, and a mismatched pair installed would break the
    /// handshake for this hostname outright.
    #[post("/projects/:project/services/:service/ports/:port/certificate")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_port_certificate(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let segments = super::auth::segments(&path);
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let here = format!(
            "/projects/{}/services/{}/settings",
            project.slug, service.slug
        );

        let ["projects", _, "services", _, "ports", port_id, "certificate"] = segments.as_slice()
        else {
            return Ok(see_other(&here));
        };

        // This service's port, and one that actually has a name to
        // certify. An id from another page must not repoint somebody
        // else's certificate.
        let Some(hostname) = ports::of_service(&self.state.database, &service.id)
            .await?
            .into_iter()
            .find(|port| port.id == *port_id)
            .and_then(|port| port.hostname)
        else {
            return Ok(see_other(&here));
        };

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let renew_with = match field(&form, "renew_with") {
            "self_signed" => crate::edge::policy::RenewWith::SelfSigned,
            "file" => {
                let cert_path = field(&form, "cert_path").trim().to_string();
                let key_path = field(&form, "key_path").trim().to_string();
                if cert_path.is_empty() || key_path.is_empty() {
                    return Ok(back_with_error(
                        &here,
                        "reading from files needs both a certificate and a key",
                    ));
                }
                if let Err(error) = crate::edge::certs::from_files(&hostname, &cert_path, &key_path)
                {
                    return Ok(back_with_error(&here, &error.to_string()));
                }
                crate::edge::policy::RenewWith::File {
                    cert_path,
                    key_path,
                }
            }
            _ => crate::edge::policy::RenewWith::Acme,
        };

        // Choosing the default forgets rather than records it, so the
        // stored answer cannot outlive the config that produced it.
        let stored =
            if renew_with == crate::edge::policy::RenewWith::default_for(&self.state.config) {
                crate::edge::policy::clear(&self.state.database, &hostname).await
            } else {
                crate::edge::policy::set(&self.state.database, &hostname, &renew_with).await
            };
        if let Err(error) = stored {
            return Ok(back_with_error(&here, &error.to_string()));
        }
        self.state.certificates.now();

        Ok(see_other(&here))
    }

    /// Deploy one release — the newest, or an older one, which is
    /// what a rollback is.
    #[post("/projects/:project/services/:service/releases/:release/deploy")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn deploy_release(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let segments = super::auth::segments(&path);
        let Some((project, service, back)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let here = format!("/projects/{}/services/{}", project.slug, service.slug);
        let _ = back;

        let Some(id) = segments.get(5) else {
            return Ok(see_other(&here));
        };
        let Some(release) = releases::find(&self.state.database, id).await? else {
            return Ok(see_other(&here));
        };
        // Only this service's releases: an id from another page must
        // not deploy somebody else's image here.
        if release.service_id != service.id {
            return Ok(see_other(&here));
        }

        // Enqueued, not run. Pulling an image inside this POST is what
        // held the request open long enough for somebody to reload and
        // be offered "confirm form resubmission".
        self.enqueue(&service.id, Some(&release.id)).await;
        Ok(see_other(&here))
    }

    /// What tag to watch, and whether a push goes out on its own.
    #[post("/projects/:project/services/:service/tracking")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn tracking(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        // Back to the page the form is on, not the one it used to
        // share with the service's state.
        let here = format!(
            "/projects/{}/services/{}/settings",
            project.slug, service.slug
        );

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let tag = field(&form, "track_tag");

        services::set_tracking(
            &self.state.database,
            &service.id,
            (!tag.is_empty()).then_some(tag),
            checked(&form, "auto_deploy"),
        )
        .await?;
        Ok(see_other(&here))
    }

    /// Change the environment, and redeploy with it.
    #[post("/projects/:project/services/:service/env")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_env(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        // Back to the page the form is on, not the one it used to
        // share with the service's state.
        let here = format!(
            "/projects/{}/services/{}/settings",
            project.slug, service.slug
        );
        let account = signed_in(&self.auth);

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let env = match parse_env(field(&form, "env")) {
            Ok(env) => env,
            Err(message) => return Ok(back_with_error(&here, &message)),
        };

        self.apply_env(&project, &service, env, account.map(|a| a.id), "edit")
            .await?;
        Ok(see_other(&here))
    }

    /// Put an earlier set of values back.
    ///
    /// Independent of releases on purpose: the usual case is "this
    /// build is bad, run the previous one, keep the settings I fixed
    /// since" — and the other way round just as often.
    #[post("/projects/:project/services/:service/env/:revision/revert")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revert_env(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let segments = super::auth::segments(&path);
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        // Back to the page the form is on, not the one it used to
        // share with the service's state.
        let here = format!(
            "/projects/{}/services/{}/settings",
            project.slug, service.slug
        );
        let account = signed_in(&self.auth);

        let Some(id) = segments.get(5) else {
            return Ok(see_other(&here));
        };
        let Some(revision) = config_history::find(&self.state.database, id).await? else {
            return Ok(see_other(&here));
        };
        if revision.service_id != service.id {
            return Ok(see_other(&here));
        }

        let env: Vec<(String, String)> = revision.env.into_iter().collect();
        self.apply_env(&project, &service, env, account.map(|a| a.id), "revert")
            .await?;
        Ok(see_other(&here))
    }

    /// Start (or restart) a service's container.
    #[post("/projects/:project/services/:service/deploy")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn deploy(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, back)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let back = returning_to(request, &back).await;

        // The failure lands on the row, not in this redirect: the job
        // outlives the answer, and a reason carried in a query string
        // is one the next click loses anyway.
        let _ = project;
        self.enqueue(&service.id, None).await;
        Ok(see_other(&back))
    }

    /// Stop a service and take it off its project's network.
    #[post("/projects/:project/services/:service/stop")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn stop(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, back)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let back = returning_to(request, &back).await;

        match self.state.deployer.stop(&project, &service).await {
            Ok(()) => Ok(see_other(&back)),
            Err(error) => Ok(back_with_error(&back, &error.to_string())),
        }
    }

    /// Remove a service, and the container behind it.
    #[post("/projects/:project/services/:service/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let Some((project, service, back)) = self.locate(request.uri().path()).await? else {
            // Already gone is the outcome they asked for.
            return Ok(see_other("/"));
        };

        // The container first. A row deleted while its container runs
        // is a container nothing will ever clean up: the id is derived
        // from the row, so losing the row loses the handle.
        self.state.deployer.tear_down(&project, &service).await;
        services::delete(&self.state.database, &service.id).await?;
        Ok(see_other(&back))
    }
    /// Throw a service somebody else placed here off this machine.
    ///
    /// The one thing this node's operator can always do to something
    /// they did not create — the machine is theirs even when the orders
    /// are not, which is the same rule the grant follows in the other
    /// direction.
    ///
    /// **The guard is the inverse of `locate`'s.** That one refuses a
    /// foreign service because every caller of it changes something
    /// this node does not decide; this one refuses a service of *our
    /// own*, because deleting one of those is what the delete button is
    /// for and it has different consequences.
    ///
    /// The rows stay, marked evicted. They are what the next report
    /// carries, and that report is the only thing that makes the node
    /// which placed it stop asking — deleting them here would leave the
    /// authority re-sending an errand for a container this machine has
    /// just thrown out.
    #[post("/projects/:project/services/:service/evict")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn evict(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        // Not a project-level decision: what is being thrown out was
        // put here by another node, and answering to it is the node's
        // business rather than one project's.
        if !account.is_admin() {
            return Ok(see_other("/"));
        }

        let path = request.uri().path().to_string();
        let Some((project_slug, service_slug)) = service_path(&path) else {
            return Ok(see_other("/"));
        };
        let Some(project) = crate::platform::projects::all(&self.state.database)
            .await?
            .into_iter()
            .find(|project| project.slug == project_slug)
        else {
            return Ok(see_other("/"));
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, service_slug).await?
        else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{}/services/{}", project.slug, service.slug);

        // The inverse guard. A service of this node's own is deleted,
        // not evicted, and the two are not the same button.
        let Some(origin) = service.origin_node_id.clone() else {
            return Ok(back_with_error(
                &here,
                "this service was created here — deleting it is the button for that",
            ));
        };

        // The containers first: a row marked evicted while its
        // container kept running would be the page lying about the
        // machine it is describing.
        self.state.deployer.tear_down(&project, &service).await;

        for replica in crate::platform::replicas::of_service(&self.state.database, &service.id)
            .await?
            .into_iter()
            .filter(|replica| replica.is_here())
        {
            crate::platform::replicas::evict(&self.state.database, &replica.id).await?;
        }

        tracing::info!(service = %service.slug, %origin, "evicted a service placed from elsewhere");
        Ok(see_other(&here))
    }

    /// How many copies of this service run, and where each one goes.
    ///
    /// One form for both, because they are one decision: a replica is a
    /// slot with a node in it, and asking for three of them while
    /// naming where two go would be a page that could be submitted
    /// half-answered.
    ///
    /// The errand for a replica placed elsewhere is queued here rather
    /// than by a background pass. The operator pressed a button, and
    /// the thing they asked for should exist before the response comes
    /// back — a queue that noticed later would leave the page showing
    /// nothing happened.
    #[post("/projects/:project/services/:service/placement")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_placement(&self, request: Request) -> RestResult<Response> {
        // Needed for the credential below: a push token is minted *by*
        // somebody, and the column that says who is a foreign key to an
        // account. A placeholder there fails the insert, which
        // `dispatch` then reports as a warning nobody reads — so the
        // errand simply never appeared.
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some((project, service, back)) = self.locate(&path).await? else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{}/services/{}", project.slug, service.slug);
        let _ = back;

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let placements =
            crate::platform::replicas::of_service(&self.state.database, &service.id).await?;
        // Every node this service is on *before* anything moves. A node
        // losing its last replica has to be told, and after the move
        // there is nothing left pointing at it — so the set is taken
        // now. Without this a replica brought home kept running there
        // for ever, invisible from both sides.
        let mut touched: std::collections::BTreeSet<String> = placements
            .iter()
            .filter_map(|replica| replica.node_id.clone())
            .collect();

        // Where each existing replica should be. An unnamed slot keeps
        // what it had: a form that arrived without a field is a form
        // that said nothing about it, not one asking for "here".
        for replica in &placements {
            let Some(chosen) = form.get(&format!("slot-{}", replica.slot)) else {
                continue;
            };
            let chosen = chosen.trim();
            let wanted = (!chosen.is_empty()).then_some(chosen);
            if wanted == replica.node_id.as_deref() {
                continue;
            }
            if let Some(node) = wanted {
                if crate::network::find(&self.state.database, node)
                    .await?
                    .is_none()
                {
                    return Ok(back_with_error(&here, "no such node"));
                }
                touched.insert(node.to_string());
            }
            crate::platform::replicas::move_to(&self.state.database, &replica.id, wanted).await?;
        }

        // Then the count. After the moves, so that a form which both
        // moves slot 2 away and drops it removes the one it moved
        // rather than a different one.
        let wanted: u32 = field(&form, "replicas").parse().unwrap_or(1);
        if !(1..=16).contains(&wanted) {
            return Ok(back_with_error(
                &here,
                "a service runs between 1 and 16 replicas",
            ));
        }
        if let Err(error) = self.resize(&service.id, wanted).await {
            return Ok(back_with_error(&here, &error));
        }

        self.dispatch(&project, &service, &account.id, touched)
            .await?;
        Ok(see_other(&here))
    }
}

impl ServiceApi {
    /// Store an environment, record the revision, and redeploy.
    ///
    /// One place, because the three have to happen together: values
    /// stored without a revision are values nobody can undo, and
    /// values stored without a redeploy are values the container does
    /// not have.
    async fn apply_env(
        &self,
        project: &crate::platform::projects::Project,
        service: &services::Service,
        env: Vec<(String, String)>,
        by: Option<String>,
        reason: &str,
    ) -> RestResult<()> {
        let map: std::collections::BTreeMap<String, String> = env.into_iter().collect();
        services::set_env(&self.state.database, &service.id, &map).await?;
        config_history::record(
            &self.state.database,
            &service.id,
            &map,
            by.as_deref(),
            reason,
        )
        .await?;

        let updated = services::Service {
            env: map,
            ..service.clone()
        };

        // Redeployed with the release it is *already running*, not with
        // its image reference. Those differ the moment a tag moves —
        // and then changing a variable would quietly bring in whatever
        // was pushed since, which is precisely what releases exist to
        // prevent. Found by changing an environment variable and
        // watching the service jump to a different image.
        let current = releases::of_service(&self.state.database, &service.id)
            .await?
            .into_iter()
            .find(|release| release.deployed_at.is_some());

        let _ = project;
        self.enqueue(
            &updated.id,
            current.as_ref().map(|release| release.id.as_str()),
        )
        .await;
        Ok(())
    }

    /// Ask for a deployment and answer the browser.
    ///
    /// The reason a failure is not returned here: by the time one
    /// exists this request is long finished. `deploy` writes it to the
    /// service row, which is what the page reads — see
    /// [`crate::deploy::jobs`].
    async fn enqueue(&self, service_id: &str, release_id: Option<&str>) {
        let command = crate::deploy::jobs::DeployService {
            service_id: service_id.to_string(),
            release_id: release_id.map(str::to_string),
        };
        if let Err(error) = wabot::async_jobs::run_command(&self.state.container, &command).await {
            // Nothing else can report this: there is no job to carry
            // the reason, because making one is what failed.
            tracing::error!(%service_id, %error, "could not queue a deployment");
        }
    }

    /// The project and service this request is about, and where to send
    /// the browser afterwards.
    ///
    /// `None` when the session is missing or either slug names nothing
    /// — the caller turns that into a redirect, because at this point
    /// there is no page to render an error onto.
    ///
    /// Takes the path rather than the request: holding a `&Request`
    /// across an `await` would need `Body: Sync`, which it is not, and
    /// the handler quietly stops being one axum will accept.
    /// Add slots here, or take the highest-numbered ones away.
    ///
    /// A replica on another node is refused rather than removed:
    /// nothing can tell that node to stop yet, so dropping the row
    /// would leave its container running with nothing naming it — the
    /// worst of both, and invisible from either end.
    async fn resize(&self, service_id: &str, wanted: u32) -> Result<(), String> {
        let placements = crate::platform::replicas::of_service(&self.state.database, service_id)
            .await
            .map_err(|error| error.to_string())?;

        if placements.len() as u32 > wanted {
            // Which ones go, and the order is the point. By slot number
            // alone, lowering a service with a dead #2 and a healthy #3
            // removes the healthy one and keeps the corpse — so the
            // ones already thrown out where they ran go first, and only
            // then the highest-numbered.
            let mut going: Vec<_> = placements.iter().collect();
            going.sort_by_key(|replica| (replica.evicted(), replica.slot));
            going.reverse();
            going.truncate(placements.len() - wanted as usize);

            for replica in going {
                // An evicted copy is safe to let go wherever it ran:
                // the machine running it stopped its container, which
                // is what evicting means. A live one elsewhere is not —
                // nothing can tell that node to stop it, and dropping
                // the row would leave it running with nothing naming
                // it.
                if !replica.is_here() && !replica.evicted() {
                    return Err(format!(
                        "replica #{} runs on another node, and nothing can tell it to stop yet",
                        replica.slot
                    ));
                }
                crate::platform::replicas::remove(&self.state.database, &replica.id)
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }

        // By count, not by range: after a removal the service may hold
        // slots 1 and 3, which is already two copies. Filling
        // `1..=wanted` would put slot 2 back and undo the removal that
        // just happened — which is exactly what it did.
        crate::platform::replicas::ensure_count_here(&self.state.database, service_id, wanted)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Ask every node holding a replica of this service to run it, and
    /// deploy the ones that are here.
    ///
    /// Convergent on both sides: a `host` errand sent twice points one
    /// service at one image there, and a local deploy is the deploy the
    /// button already queues.
    async fn dispatch(
        &self,
        project: &crate::platform::projects::Project,
        service: &services::Service,
        by: &str,
        touched: std::collections::BTreeSet<String>,
    ) -> RestResult<()> {
        let placements =
            crate::platform::replicas::of_service(&self.state.database, &service.id).await?;

        // Every node that holds a replica *or has just stopped holding
        // one*. The second half is why the set is passed in: a node
        // losing its last copy is exactly the one nothing points at any
        // more, and it is the one that most needs telling.
        for node_id in touched {
            // Every slot that node holds, in the service's numbering:
            // a node running two copies is told about both in one
            // errand rather than being sent the same instruction twice
            // with no way to tell the copies apart.
            //
            // An **empty** list is a real instruction, not a reason to
            // skip: it says "you run none of this now", which is how a
            // node finds out that its last replica went home.
            let slots: Vec<u32> = placements
                .iter()
                .filter(|replica| replica.node_id.as_deref() == Some(node_id.as_str()))
                .filter(|replica| !replica.evicted())
                .map(|replica| replica.slot)
                .collect();

            if let Err(error) = self.send_there(project, service, &node_id, by, slots).await {
                // Reported and not fatal: the other placements are
                // still worth making, and the page shows a replica
                // nobody has answered for.
                tracing::warn!(node = %node_id, %error, "could not queue a host errand");
            }
        }

        if placements.iter().any(|replica| replica.is_here()) {
            self.enqueue(&service.id, None).await;
        }
        Ok(())
    }

    /// One `host` errand, with a credential for this node's registry.
    async fn send_there(
        &self,
        project: &crate::platform::projects::Project,
        service: &services::Service,
        node_id: &str,
        by: &str,
        slots: Vec<u32>,
    ) -> Result<(), String> {
        let Some(registry) = crate::platform::registry_credentials::host_of(&service.image) else {
            return Err(
                "that image does not name a registry, so there is nothing to pull it from".into(),
            );
        };

        let (_, secret) = crate::platform::tokens::create(
            &self.state.database,
            &project.id,
            &format!("errand to {node_id}"),
            by,
        )
        .await
        .map_err(|error| error.to_string())?;

        let payload = serde_json::to_value(crate::network::errand::Host {
            project: project.name.clone(),
            service: service.name.clone(),
            image: service.image.clone(),
            registry,
            username: "errand".into(),
            secret,
            env: service.env.clone(),
            port: None,
            slots,
        })
        .map_err(|error| error.to_string())?;

        crate::network::errand::queue(
            &self.state.database,
            node_id,
            crate::network::errand::Kind::Host,
            &payload,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    async fn locate(
        &self,
        path: &str,
    ) -> RestResult<
        Option<(
            crate::platform::projects::Project,
            services::Service,
            String,
        )>,
    > {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(None);
        };
        let Some((project_slug, service_slug)) = service_path(path) else {
            return Ok(None);
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, project_slug).await?
        else {
            return Ok(None);
        };
        // Every caller of this mutates something. A viewer gets the
        // same answer as a stranger, which is the same answer as a
        // service that does not exist.
        if !allowed.may_deploy() {
            return Ok(None);
        }
        let found = services::in_project(&self.state.database, &project.id, service_slug).await?;

        // A service that arrived on an errand is administered from the
        // node that sent it. Every caller of this changes something, so
        // a foreign one answers the same way a stranger's does — the
        // one thing this node's operator can do to it is throw it out,
        // and that has its own path through the danger zone rather than
        // going through here.
        //
        // The check is on the *service*, not only the project: a
        // project that arrived from elsewhere holds only foreign
        // services, but reading it off the row that is actually being
        // changed is the one that cannot be wrong.
        let found = found.filter(|service| service.is_ours());

        let back = format!("/projects/{}", project.slug);
        Ok(found.map(|service| (project, service, back)))
    }
}

/// Was a checkbox ticked?
///
/// A browser omits an unchecked box entirely rather than sending it
/// with a false value, so presence *is* the answer.
fn checked(form: &std::collections::HashMap<String, String>, name: &str) -> bool {
    form.contains_key(name)
}

/// The project and service a `/projects/…/services/…/…` path names.
///
/// `#[raw]` hands over the whole request and no extracted parameters,
/// so both slugs come out of the URI here.
fn service_path(path: &str) -> Option<(&str, &str)> {
    match super::auth::segments(path).as_slice() {
        // The service's own page, and every action under it — the
        // trailing parts differ (`deploy`, `ports`, `ports/<id>/delete`)
        // and none of them change which service is meant.
        ["projects", project, "services", service, ..]
            if !project.is_empty() && !service.is_empty() =>
        {
            Some((project, service))
        }
        _ => None,
    }
}

/// The project slug out of `/projects/{slug}/services…`.
fn project_slug(path: &str) -> Option<&str> {
    let segments = super::auth::segments(path);
    match segments.as_slice() {
        ["projects", slug, ..] if !slug.is_empty() => Some(slug),
        _ => None,
    }
}

/// An optional port from a form field.
///
/// Empty is `None` — a service that serves no traffic is a real thing.
/// Anything else has to be a port, and `0` is not one.
fn parse_port(value: &str) -> Result<Option<u16>, &'static str> {
    if value.is_empty() {
        return Ok(None);
    }
    match value.parse::<u16>() {
        Ok(0) | Err(_) => Err("a container port is a number between 1 and 65535"),
        Ok(port) => Ok(Some(port)),
    }
}

/// `KEY=value` lines into pairs.
///
/// The split is on the *first* `=` only: a value is very often a URL or
/// a base64 blob with one in it, and splitting on every `=` truncates
/// exactly the secrets somebody would least like truncated.
///
/// Blank lines and `#` comments are skipped, so a block pasted out of
/// a `.env` file works as pasted.
fn parse_env(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut pairs = Vec::new();

    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {} is not KEY=value: {line:?}", number + 1));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("line {} has no name before the =", number + 1));
        }
        if !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            return Err(format!(
                "{key:?} is not a usable variable name — letters, digits, _ and ."
            ));
        }

        // The value is trimmed but not unquoted: `KEY="v"` in a shell
        // file means `v`, and guessing wrong either way puts literal
        // quotes into a password. Whoever pastes it can see the quotes.
        pairs.push((key.to_string(), value.trim().to_string()));
    }

    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;
    use crate::platform::projects;
    use wabot::rest::axum::http::StatusCode;

    /// A project and a service to place, made the way the console does.
    async fn placed_service(console: &Console, cookie: &str) -> services::Service {
        console
            .harness
            .post("/projects")
            .header("cookie", cookie)
            .form(&[("name", "shared")])
            .send()
            .await;
        let project = projects::all(&console.database)
            .await
            .expect("projects")
            .into_iter()
            .find(|project| project.slug == "shared")
            .expect("made");
        services::create(
            &console.database,
            &project.id,
            "web",
            "hub.example.com/shared/web@sha256:abc",
            &[],
        )
        .await
        .expect("service")
    }

    /// The page you go to when a service is down was the one page you
    /// could not start it from: it showed the state and withheld the
    /// control.
    #[tokio::test]
    async fn the_service_page_offers_the_control_its_state_calls_for() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        placed_service(&console, &cookie).await;

        let body = console
            .harness
            .get("/projects/shared/services/web")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            body.contains("/projects/shared/services/web/deploy"),
            "no way to start it: {body}"
        );
        assert!(
            body.contains("/projects/shared/services/web/stop"),
            "{body}"
        );
    }

    /// The badge said "Deploying" until somebody reloaded, on the one
    /// page they were watching to find out when it stopped. It joins
    /// the project's stream rather than opening one of its own: the
    /// island writes by service id, and this page has one of each cell.
    #[tokio::test]
    async fn the_service_page_comes_alive_on_the_projects_stream() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;

        let page = console
            .ui
            .with_header("cookie", cookie)
            .get("/projects/shared/services/web")
            .await;

        assert!(page.has_island("project-live"), "{}", page.html());
        assert_eq!(
            page.island_props("project-live"),
            Some(serde_json::json!({ "project": "shared" })),
            "the client needs the project to open the stream"
        );

        let html = page.html();
        for hook in [
            format!("data-state=\"{}\"", service.id),
            format!("data-address=\"{}\"", service.id),
        ] {
            assert!(
                html.contains(&hook),
                "nothing for the stream to write into: {hook}"
            );
        }
    }

    /// The same two controls are on the list and on the service's own
    /// page. Sending somebody back to the list from the page they were
    /// reading is the console deciding they meant to leave.
    #[tokio::test]
    async fn a_control_returns_to_the_page_it_was_pressed_on() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        placed_service(&console, &cookie).await;

        let response = console
            .harness
            .post("/projects/shared/services/web/stop")
            .header("cookie", &cookie)
            .form(&[("from", "/projects/shared/services/web")])
            .send()
            .await;
        // The reason rides along when there is one — no containerd in
        // a test — but the *page* is what this pins.
        let location = response.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/projects/shared/services/web"),
            "{location}"
        );

        // A path this console put in its own markup, never somewhere
        // else's: that would let another page choose where a submit
        // lands.
        let elsewhere = console
            .harness
            .post("/projects/shared/services/web/stop")
            .header("cookie", &cookie)
            .form(&[("from", "https://elsewhere.example/")])
            .send()
            .await;
        let location = elsewhere.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/projects/shared?") || location == "/projects/shared",
            "somewhere else chose where the submit landed: {location}"
        );
    }

    /// By slot number alone, lowering a service with a dead #2 and a
    /// healthy #3 removes the healthy one and keeps the corpse.
    #[tokio::test]
    async fn lowering_the_count_lets_the_evicted_go_first() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;

        // #2 thrown out where it ran, #3 alive and here.
        let two = crate::platform::replicas::place(
            &console.database,
            &service.id,
            Some("nd-elsewhere1"),
            2,
        )
        .await
        .expect("placed");
        crate::platform::replicas::evict(&console.database, &two.id)
            .await
            .expect("evicted");
        crate::platform::replicas::place(&console.database, &service.id, None, 3)
            .await
            .expect("placed");

        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "2")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let left: Vec<u32> = crate::platform::replicas::of_service(&console.database, &service.id)
            .await
            .expect("replicas")
            .into_iter()
            .map(|replica| replica.slot)
            .collect();
        assert_eq!(
            left,
            vec![1, 3],
            "the corpse was kept and the live one killed"
        );
    }

    /// A service placed here by another node is not this node's to
    /// start or stop — the same boundary every mutation goes through,
    /// and the page must not offer what the POST will refuse.
    #[tokio::test]
    async fn a_foreign_service_is_offered_no_controls() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        services::set_origin(&console.database, &service.id, "nd-elsewhere1")
            .await
            .expect("origin");

        let body = console
            .harness
            .get("/projects/shared/services/web")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            !body.contains("/projects/shared/services/web/deploy"),
            "it offered to start somebody else's service"
        );
        assert!(body.contains("From another node"), "{body}");
    }

    /// The one thing the operator of a machine can always do to
    /// something they did not put there. The machine is theirs even
    /// when the orders are not.
    #[tokio::test]
    async fn a_service_from_another_node_can_be_thrown_off_this_one() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        services::set_origin(&console.database, &service.id, "nd-elsewhere1")
            .await
            .expect("origin");

        console
            .harness
            .post("/projects/shared/services/web/evict")
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let placements = crate::platform::replicas::of_service(&console.database, &service.id)
            .await
            .expect("replicas");
        assert!(
            placements.iter().all(|replica| replica.evicted()),
            "{placements:?}"
        );

        // The rows stay: they are what the next report carries, and
        // that report is the only thing that makes the other node stop
        // asking.
        assert!(
            services::in_project(&console.database, &service.project_id, "web")
                .await
                .expect("query")
                .is_some(),
            "the tombstone was removed, so the errand will come back"
        );
    }

    /// The guard is the inverse of `locate`'s: a service this node made
    /// itself is *deleted*, not evicted, and the two buttons do not
    /// mean the same thing.
    #[tokio::test]
    async fn a_service_of_this_nodes_own_is_not_evictable() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;

        let response = console
            .harness
            .post("/projects/shared/services/web/evict")
            .header("cookie", &cookie)
            .send()
            .await;

        let location = response.header("location").unwrap_or_default();
        assert!(location.contains("error="), "{location}");
        assert!(
            crate::platform::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas")
                .iter()
                .all(|replica| !replica.evicted()),
            "somebody's own service was evicted"
        );
    }

    /// Throwing another node's work off this machine is the node's
    /// business, not one project's.
    #[tokio::test]
    async fn only_an_admin_may_evict() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let service = placed_service(&console, &admin).await;
        services::set_origin(&console.database, &service.id, "nd-elsewhere1")
            .await
            .expect("origin");
        let member = console.joined_as(&admin, "member").await;

        let response = console
            .harness
            .post("/projects/shared/services/web/evict")
            .header("cookie", &member)
            .send()
            .await;

        assert_eq!(response.header("location"), Some("/"));
        assert!(
            crate::platform::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas")
                .iter()
                .all(|replica| !replica.evicted()),
            "a member evicted another node's service"
        );
    }

    /// The page the whole goal hangs off: a service is administered
    /// from the node that created it, and that is where somebody says
    /// how many copies run and where each of them goes.
    #[tokio::test]
    async fn a_replica_is_placed_on_another_node_from_the_service_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::network::save(
            &console.database,
            &crate::network::Node {
                id: "nd-elsewhere1".into(),
                name: "alpine.example".into(),
                kind: crate::network::Kind::Private,
                endpoint: None,
                public_key: Some("k".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: None,
            },
        )
        .await
        .expect("node");

        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "2")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[
                ("replicas", "2"),
                ("slot-1", ""),
                ("slot-2", "nd-elsewhere1"),
            ])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let placements = crate::platform::replicas::of_service(&console.database, &service.id)
            .await
            .expect("replicas");
        assert_eq!(placements.len(), 2);
        assert!(placements[0].is_here());
        assert_eq!(placements[1].node_id.as_deref(), Some("nd-elsewhere1"));

        // And that node has been asked to run it, with something to
        // pull the image with.
        let queued = crate::network::errand::all(&console.database)
            .await
            .expect("errands");
        assert_eq!(queued.len(), 1, "{queued:?}");
        assert_eq!(queued[0].node_id, "nd-elsewhere1");

        let waiting = crate::network::errand::waiting(&console.database, "nd-elsewhere1")
            .await
            .expect("waiting");
        let host: crate::network::errand::Host =
            serde_json::from_value(waiting[0].payload.clone()).expect("a host errand");
        assert_eq!(host.service, "web");
        assert!(!host.secret.is_empty(), "nothing to pull it with");
    }

    /// Bringing a replica home has to tell the node it left.
    ///
    /// This shipped in v0.5.0 as a leak: the row moved and nothing was
    /// sent, so that node kept the container running for ever — and a
    /// report about it is ignored by design, because the row is local
    /// now. Invisible from both sides, which is the worst shape a bug
    /// can have here.
    #[tokio::test]
    async fn bringing_a_replica_home_tells_the_node_it_left() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::network::save(
            &console.database,
            &crate::network::Node {
                id: "nd-elsewhere1".into(),
                name: "alpine.example".into(),
                kind: crate::network::Kind::Private,
                endpoint: None,
                public_key: Some("k".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: None,
            },
        )
        .await
        .expect("node");

        // Two replicas, the second one away.
        for form in [
            vec![("replicas", "2")],
            vec![("replicas", "2"), ("slot-2", "nd-elsewhere1")],
        ] {
            console
                .harness
                .post("/projects/shared/services/web/placement")
                .header("cookie", &cookie)
                .form(&form)
                .send()
                .await
                .assert_status(StatusCode::SEE_OTHER);
        }
        crate::network::errand::waiting(&console.database, "nd-elsewhere1")
            .await
            .expect("collected");

        // And back home.
        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "2"), ("slot-1", ""), ("slot-2", "")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let placements = crate::platform::replicas::of_service(&console.database, &service.id)
            .await
            .expect("replicas");
        assert!(placements.iter().all(|replica| replica.is_here()));

        // The node it left is told, and told it runs *none* of it —
        // which is the instruction that stops its container.
        // The newest one, and it says it runs *none* of this — which
        // is the instruction that stops its container. Collecting does
        // not settle an errand, so the earlier one is still pending
        // beside it; what matters is the last thing it was told.
        let waiting = crate::network::errand::waiting(&console.database, "nd-elsewhere1")
            .await
            .expect("waiting");
        let latest = waiting.last().expect("the node it left was told nothing");
        let host: crate::network::errand::Host =
            serde_json::from_value(latest.payload.clone()).expect("a host errand");
        assert!(
            host.slots.is_empty(),
            "it was told to keep running {:?}",
            host.slots
        );
    }

    /// Removing a replica that runs somewhere else would leave its
    /// container running with nothing naming it — invisible from both
    /// ends. Refused with the reason until something can tell that node
    /// to stop.
    #[tokio::test]
    async fn a_replica_on_another_node_cannot_be_dropped_from_here_yet() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::platform::replicas::place(&console.database, &service.id, Some("nd-elsewhere1"), 2)
            .await
            .expect("placed");

        let response = console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "1")])
            .send()
            .await;

        let location = response.header("location").unwrap_or_default();
        assert!(location.contains("error="), "{location}");
        assert_eq!(
            crate::platform::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas")
                .len(),
            2,
            "it was dropped anyway"
        );
    }

    /// A service that arrived on an errand is administered from the
    /// node that sent it, and this is the boundary rather than a hidden
    /// button: every mutation goes through `locate`, so a foreign
    /// service answers the way a stranger's does.
    ///
    /// This was claimed in the commit that added the guard and was not
    /// actually there — a text replacement whose anchor did not exist
    /// failed silently. Hence the test, and hence this note.
    #[tokio::test]
    async fn a_service_from_another_node_cannot_be_changed_here() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        services::set_origin(&console.database, &service.id, "nd-elsewhere1")
            .await
            .expect("origin");

        // Stopping is the smallest mutation there is, and it goes
        // through the same door as every other.
        let response = console
            .harness
            .post("/projects/shared/services/web/stop")
            .header("cookie", &cookie)
            .send()
            .await;
        // "No such service" is the answer on purpose: a foreign one
        // reads exactly the way a stranger's does, which is what makes
        // this a boundary rather than a hidden button.
        let location = response.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/?error="),
            "a foreign service was changed from here: {location}"
        );

        let stored = services::in_project(&console.database, &service.project_id, "web")
            .await
            .expect("query")
            .expect("there");
        assert_eq!(
            stored.desired_state,
            services::DesiredState::Running,
            "the intent was changed"
        );
    }

    async fn project(console: &Console, cookie: &str) -> String {
        console
            .harness
            .post("/projects")
            .header("cookie", cookie)
            .form(&[("name", "My API")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        "my-api".to_string()
    }

    #[tokio::test]
    async fn a_service_is_created_and_listed() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", &cookie)
            .form(&[
                ("name", "web"),
                ("image", "docker.io/library/nginx:alpine"),
                ("env", "LOG_LEVEL=info\nDSN=postgres://a=b"),
            ])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(
            response.header("location"),
            Some("/projects/my-api/services/web"),
            "straight to the service, where its ports are declared"
        );

        let body = console
            .harness
            .get("/projects/my-api")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("web"), "{body}");
        // Not the image: the list dropped that column. Which image a
        // service runs is on the service's own page, where somebody
        // deciding about it is already looking.
        assert!(!body.contains("nginx:alpine"), "{body}");

        // The value with an `=` in it survived the form.
        let stored = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        assert_eq!(
            stored.env.get("DSN").map(String::as_str),
            Some("postgres://a=b")
        );
    }

    /// A deployment that could not happen has to say so on the page,
    /// not just in a log. This runs where containerd is not — a
    /// developer's machine — which is exactly the failure an operator
    /// meets when the socket is down.
    ///
    /// The reason is put on the row by `Deployer::deploy`, which is
    /// what the job calls. Driven directly here rather than through the
    /// POST: creating a service now *queues* a deployment, and this
    /// harness has no job runner, so going through the form would test
    /// that nothing happened.
    #[tokio::test]
    async fn a_deployment_that_fails_says_why_on_the_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;

        console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let stored = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        let found = crate::platform::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("present");

        let deployer = crate::deploy::Deployer::new(
            console.database.clone(),
            &crate::config::Config::default(),
        );
        let _ = deployer.deploy(&found, &stored).await;

        let stored = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        let failure = crate::platform::replicas::of_service(&console.database, &stored.id)
            .await
            .expect("replicas")
            .into_iter()
            .next()
            .and_then(|replica| replica.last_error)
            .expect("the reason was recorded");
        assert!(
            failure.contains("containerd"),
            "the reason names the runtime: {failure}"
        );
        assert!(
            crate::platform::replicas::of_service(&console.database, &stored.id)
                .await
                .expect("replicas")
                .into_iter()
                .all(|replica| replica.address.is_none()),
            "nothing to route to"
        );

        let body = console
            .harness
            .get("/projects/my-api")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("class=\"failure\""), "the row is rendered");
        assert!(body.contains("containerd"), "with the reason in it");
    }

    /// The join the node found broken: a queued deployment has to
    /// reach `Deployer::deploy` and leave its reason on the row.
    ///
    /// It did not, in production, because nothing registered
    /// `SqliteDatabase` in the container and the handler could not be
    /// built — the worker panicked, the button still answered, and the
    /// service stayed as it was. This is the test that would have
    /// caught it.
    #[tokio::test]
    async fn a_queued_deployment_reaches_the_row() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/deploy"))
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        // Polled, not slept: the job runs on a spawned task, and a
        // fixed wait is either flaky or slow.
        let mut reason = None;
        for _ in 0..150 {
            let stored = services::all(&console.database, None)
                .await
                .expect("query")
                .pop()
                .expect("one service");
            if let Some(found) =
                crate::platform::replicas::of_service(&console.database, &stored.id)
                    .await
                    .expect("replicas")
                    .into_iter()
                    .next()
                    .and_then(|replica| replica.last_error)
            {
                reason = Some(found);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let reason = reason.expect("the handler ran and recorded a reason");
        assert!(
            reason.contains("containerd"),
            "which names the runtime: {reason}"
        );
    }

    /// The POST answers before the work happens. Holding the request
    /// open for an image pull is what offered somebody "confirm form
    /// resubmission" when they reloaded.
    #[tokio::test]
    async fn deploying_answers_without_waiting_for_the_deployment() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("{page}/deploy"))
            .header("cookie", &cookie)
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);

        // The job is spawned, not awaited, so the row is still
        // untouched the instant the answer arrives. What eventually
        // lands there is the previous test's subject.
        let stored = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        assert!(
            crate::platform::replicas::of_service(&console.database, &stored.id)
                .await
                .expect("replicas")
                .into_iter()
                .all(|replica| replica.address.is_none()),
            "nothing was deployed inline"
        );
    }

    #[tokio::test]
    async fn a_bad_image_returns_to_the_form_with_the_reason() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "https://example.com/nginx")])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").expect("redirected");
        assert!(
            location.starts_with("/projects/my-api/services/new?error="),
            "{location}"
        );
    }

    #[tokio::test]
    async fn creating_a_service_needs_a_session() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;
        assert_eq!(response.header("location"), Some("/sign-in"));
        assert!(services::all(&console.database, None)
            .await
            .expect("query")
            .is_empty());
    }

    #[tokio::test]
    async fn a_service_can_be_deleted_from_the_project_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;
        console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;

        let response = console
            .harness
            .post(&format!("/projects/{slug}/services/web/delete"))
            .header("cookie", &cookie)
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/projects/my-api"));
        assert!(services::all(&console.database, None)
            .await
            .expect("query")
            .is_empty());
    }

    /// A service of the same name in another project must not be the
    /// one that disappears.
    #[tokio::test]
    async fn deleting_names_the_service_inside_its_own_project() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        for name in ["one", "two"] {
            console
                .harness
                .post("/projects")
                .header("cookie", &cookie)
                .form(&[("name", name)])
                .send()
                .await;
            console
                .harness
                .post(&format!("/projects/{name}/services"))
                .header("cookie", &cookie)
                .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
                .send()
                .await;
        }

        console
            .harness
            .post("/projects/one/services/web/delete")
            .header("cookie", &cookie)
            .send()
            .await;

        let left = services::all(&console.database, None).await.expect("query");
        assert_eq!(left.len(), 1, "only one went");
        let survivor = projects::find(&console.database, "two")
            .await
            .expect("query")
            .expect("present");
        assert_eq!(left[0].project_id, survivor.id, "and it was the right one");
    }

    async fn service(console: &Console, cookie: &str) -> String {
        let slug = project(console, cookie).await;
        console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;
        "/projects/my-api/services/web".to_string()
    }

    /// The split: one page says what the service is doing, the other
    /// changes it. A form that drifts back onto the service page puts
    /// a text box next to the state somebody is watching, which is the
    /// crowding this separation exists to undo.
    #[tokio::test]
    async fn the_service_page_reads_and_the_settings_page_writes() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let reading = console
            .harness
            .get(&page)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        for form in [
            "/tracking\"",
            "/env\"",
            "name=\"container_port\"",
            "name=\"env\"",
        ] {
            assert!(
                !reading.contains(form),
                "{form} is still on the service page: {reading}"
            );
        }
        assert!(reading.contains("/settings"), "and it links to them");

        let writing = console
            .harness
            .get(&format!("{page}/settings"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        for form in [
            "/tracking\"",
            "/env\"",
            "name=\"container_port\"",
            "name=\"env\"",
        ] {
            assert!(writing.contains(form), "{form} is missing: {writing}");
        }
    }

    /// Each form comes back to the page it is on. Landing on the
    /// service page after saving means the next edit starts with a
    /// click somebody should not have had to make.
    #[tokio::test]
    async fn saving_a_setting_stays_on_the_settings_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;
        let settings = format!("{page}/settings");

        for (action, form) in [
            ("tracking", vec![("track_tag", "latest")]),
            ("env", vec![("env", "A=1")]),
            ("ports", vec![("container_port", "8080")]),
        ] {
            let response = console
                .harness
                .post(&format!("{page}/{action}"))
                .header("cookie", &cookie)
                .form(&form)
                .send()
                .await;
            let location = response.header("location").expect("redirected");
            assert!(
                location.starts_with(&settings),
                "{action} went to {location}"
            );
        }
    }

    /// A viewer may read the service and change nothing, so the page
    /// made of nothing but controls is not a page for them.
    #[tokio::test]
    async fn a_viewer_asking_for_settings_gets_the_service() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let page = service(&console, &admin).await;
        let member = console.joined_as(&admin, "watcher").await;

        let watcher = crate::accounts::all(&console.database)
            .await
            .expect("query")
            .into_iter()
            .find(|account| account.username == "watcher")
            .expect("joined");
        let project = crate::platform::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("present");
        access::grant(
            &console.database,
            &watcher.id,
            &project.id,
            crate::accounts::roles::ProjectRole::Viewer,
        )
        .await
        .expect("granted");

        let response = console
            .harness
            .get(&format!("{page}/settings"))
            .header("cookie", &member)
            .send()
            .await;
        assert_eq!(
            response.header("location"),
            Some(page.as_str()),
            "sent to the page they may read"
        );
    }

    /// A hostname declared through `ports::create` rather than the
    /// form: the form checks DNS first, and no name resolves to a test.
    async fn hostname_on(console: &Console, hostname: &str) -> String {
        let project = crate::platform::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("present");
        let service = services::in_project(&console.database, &project.id, "web")
            .await
            .expect("query")
            .expect("present");
        let port = ports::create(&console.database, &service.id, 80, false, Some(hostname))
            .await
            .expect("port");
        port.id
    }

    /// Every name the node serves gets the same three answers. The
    /// node's own domain had a page first; that is the only thing
    /// special about it.
    #[tokio::test]
    async fn a_service_hostname_gets_its_own_certificate_control() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;
        let port_id = hostname_on(&console, "api.example.com").await;

        let body = console
            .harness
            .get(&format!("{page}/settings"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("api.example.com"), "{body}");
        assert!(
            body.contains(&format!("{page}/ports/{port_id}/certificate")),
            "the form posts for this port: {body}"
        );
        assert!(
            body.contains("Read from files on this node"),
            "and offers all three: {body}"
        );
    }

    /// The hostname field is the case that could not be done in HTML
    /// alone: `required` only when the HTTPS box is ticked. The field
    /// is still in the markup and the server still refuses an empty one
    /// — the attribute is a courtesy, not the check.
    #[tokio::test]
    async fn the_hostname_field_is_conditioned_on_the_https_box() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;
        hostname_on(&console, "api.example.com").await;
        // Without a domain the node offers no hostname field at all —
        // it cannot check that a name points here, so there is nothing
        // to type. That branch is a different test.
        crate::node::settings::set_domain(&console.database, Some("node.example.com"))
            .await
            .expect("domain");

        let body = console
            .harness
            .get(&format!("{page}/settings"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            body.contains("data-required-when=\"https\""),
            "required only with the box: {body}"
        );
        super::super::nodes::tests::conditions_name_real_controls(&body);
    }

    /// Refused while there is somebody to tell. A pair stored and found
    /// wrong later fails on every pass of the renewal loop, with the
    /// reason in a journal nobody is reading.
    #[tokio::test]
    async fn a_file_pair_for_another_name_is_refused_and_not_stored() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;
        let port_id = hostname_on(&console, "api.example.com").await;

        let dir = tempfile::tempdir().expect("tempdir");
        let key = rcgen::KeyPair::generate().expect("key");
        let certificate = rcgen::CertificateParams::new(vec!["other.example.com".to_string()])
            .expect("params")
            .self_signed(&key)
            .expect("sign");
        let cert_path = dir.path().join("x.crt");
        let key_path = dir.path().join("x.key");
        std::fs::write(&cert_path, certificate.pem()).expect("write");
        std::fs::write(&key_path, key.serialize_pem()).expect("write");

        let response = console
            .harness
            .post(&format!("{page}/ports/{port_id}/certificate"))
            .header("cookie", &cookie)
            .form(&[
                ("renew_with", "file"),
                ("cert_path", cert_path.to_str().expect("path")),
                ("key_path", key_path.to_str().expect("path")),
            ])
            .send()
            .await;
        let location = response.header("location").expect("redirected");
        assert!(location.contains("error="), "it said no: {location}");

        assert_eq!(
            crate::edge::policy::for_name(
                &console.database,
                &crate::config::Config::default(),
                "api.example.com",
            )
            .await
            .renew_with,
            crate::edge::policy::RenewWith::Acme,
            "and stored nothing"
        );
    }

    /// The default answer: a service exposes nothing.
    #[tokio::test]
    async fn a_new_service_exposes_nothing() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let body = console
            .harness
            .get(&page)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("exposes nothing"), "{body}");
    }

    #[tokio::test]
    async fn a_plain_port_is_reachable_from_the_project_only() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[("container_port", "8080")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let stored = ports::all(&console.database)
            .await
            .expect("query")
            .pop()
            .expect("one port");
        assert_eq!(stored.container_port, 8080);
        assert_eq!(stored.host_port, None, "not published");
        assert_eq!(stored.hostname, None, "no hostname");

        let body = console
            .harness
            .get(&page)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("the project only"), "{body}");
    }

    #[tokio::test]
    async fn publishing_gives_the_port_a_node_port() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[("container_port", "5432"), ("publish", "1")])
            .send()
            .await;

        let stored = ports::all(&console.database)
            .await
            .expect("query")
            .pop()
            .expect("one port");
        assert!(stored.host_port.is_some(), "a node port was allocated");
    }

    /// The check that has to happen before the row is written. A
    /// hostname pointing somewhere else is a certificate request that
    /// fails and a page that never loads.
    #[tokio::test]
    async fn a_hostname_that_does_not_resolve_here_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[
                ("container_port", "80"),
                ("https", "1"),
                // Reserved by RFC 2606 so it can never resolve.
                ("hostname", "nothing.invalid"),
            ])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").expect("redirected");
        assert!(location.contains("error="), "{location}");
        assert!(
            ports::all(&console.database)
                .await
                .expect("query")
                .is_empty(),
            "nothing was written"
        );
    }

    /// A hostname without a certificate yet has to say so: the route
    /// exists the moment the port does, and the certificate follows a
    /// few seconds later. A page that claimed it was ready would be
    /// wrong for exactly the window somebody is watching.
    #[tokio::test]
    async fn a_hostname_says_when_its_certificate_has_not_arrived() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        // Written straight in: the DNS check would refuse a name that
        // does not resolve, and this test is about what the page says
        // afterwards.
        let stored = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        ports::create(
            &console.database,
            &stored.id,
            80,
            false,
            Some("api.example.com"),
        )
        .await
        .expect("port");

        let body = console
            .harness
            .get(&page)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Certificate on the way"), "{body}");
    }

    /// A node with no domain of its own cannot check that a name points
    /// at it, so `add_port` refuses every hostname on that ground. The
    /// form offered the box and a text field anyway, under a hint that
    /// said to "type a hostname already pointed here" — which could
    /// only ever come back as an error.
    #[tokio::test]
    async fn https_is_not_offered_until_the_node_has_a_domain() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let body = console
            .harness
            .get(&format!("{page}/settings"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("needs a domain"), "it says why: {body}");
        assert!(
            !body.contains("api.example.com"),
            "and offers no field to fill in: {body}"
        );

        // The disabled box is a courtesy; this is the check that counts.
        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[
                ("container_port", "80"),
                ("https", "1"),
                ("hostname", "api.example.com"),
            ])
            .send()
            .await;
        assert!(
            ports::all(&console.database)
                .await
                .expect("query")
                .is_empty(),
            "a hostname was accepted with no node domain"
        );
    }

    /// Ticking HTTPS with no hostname must not silently create a port
    /// that serves nothing.
    #[tokio::test]
    async fn https_without_a_hostname_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[("container_port", "80"), ("https", "1")])
            .send()
            .await;

        assert!(ports::all(&console.database)
            .await
            .expect("query")
            .is_empty());
    }

    /// An unticked box is not sent at all, so a hostname left in the
    /// field from a previous attempt must not become a route.
    #[tokio::test]
    async fn a_hostname_without_the_box_ticked_is_ignored() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[("container_port", "80"), ("hostname", "api.example.com")])
            .send()
            .await;

        let stored = ports::all(&console.database)
            .await
            .expect("query")
            .pop()
            .expect("one port");
        assert_eq!(stored.hostname, None);
    }

    #[tokio::test]
    async fn a_port_can_be_removed() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[("container_port", "8080")])
            .send()
            .await;
        let port = ports::all(&console.database)
            .await
            .expect("query")
            .pop()
            .expect("one port");

        console
            .harness
            .post(&format!("{page}/ports/{}/delete", port.id))
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert!(ports::all(&console.database)
            .await
            .expect("query")
            .is_empty());
    }

    #[tokio::test]
    async fn declaring_a_port_needs_a_session() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .form(&[("container_port", "8080")])
            .send()
            .await;

        assert!(ports::all(&console.database)
            .await
            .expect("query")
            .is_empty());
    }

    #[test]
    fn an_empty_port_is_a_service_that_serves_nothing() {
        assert_eq!(parse_port(""), Ok(None));
        assert_eq!(parse_port("80"), Ok(Some(80)));
        assert!(parse_port("0").is_err(), "0 is not a port");
        assert!(parse_port("70000").is_err(), "out of range");
        assert!(parse_port("eighty").is_err());
    }

    /// The mistake this parser exists to avoid: a value with an `=` in
    /// it, which is most connection strings and every base64 secret.
    #[test]
    fn a_value_may_contain_an_equals() {
        let parsed = parse_env("DSN=postgres://u:p@h/db?x=1\nKEY=YWJj==").expect("parsed");
        assert_eq!(
            parsed,
            vec![
                ("DSN".to_string(), "postgres://u:p@h/db?x=1".to_string()),
                ("KEY".to_string(), "YWJj==".to_string()),
            ]
        );
    }

    #[test]
    fn a_pasted_env_file_works_as_pasted() {
        let parsed = parse_env("# comment\n\n  LOG_LEVEL = info  \n").expect("parsed");
        assert_eq!(parsed, vec![("LOG_LEVEL".into(), "info".into())]);
    }

    #[test]
    fn a_line_that_is_not_a_pair_is_refused_by_line_number() {
        let error = parse_env("GOOD=1\nnonsense").expect_err("refused");
        assert!(error.contains("line 2"), "{error}");
    }

    #[test]
    fn a_name_the_shell_cannot_hold_is_refused() {
        let error = parse_env("not a name=1").expect_err("refused");
        assert!(error.contains("variable name"), "{error}");
        assert!(parse_env("=novalue").is_err(), "no name at all");
    }

    #[test]
    fn the_slug_comes_out_of_the_path() {
        assert_eq!(project_slug("/projects/my-api/services"), Some("my-api"));
        assert_eq!(project_slug("/projects//services"), None);
        assert_eq!(project_slug("/elsewhere"), None);
    }
}
