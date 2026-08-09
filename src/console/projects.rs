//! Projects: the list, the create form, and one project's page.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use crate::accounts::roles::ProjectRole;
use crate::deploy::Observed;
use crate::platform::{access, projects, services};

use super::auth::{
    back_with_error, field, read_form, see_other, signed_in, PageQuery, SessionMiddleware,
};
use super::shell::{Area, Frame};
use super::{layout, ConsoleState};

#[derive(Debug, Deserialize, Validate)]
pub struct ProjectQuery {
    pub project: String,
    pub error: Option<String>,
    /// Names a push token this node is holding for one read. Not the
    /// token — see `ConsoleState::reveals`.
    pub shown: Option<String>,
}

#[injectable]
pub struct ProjectPages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl ProjectPages {
    /// Everything on this node, and what state the node is in.
    #[view("/")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(
                Redirect::found(super::auth::signed_out_destination(&self.state).await).into(),
            );
        };

        // The filter is the query, not the page: a list built from
        // everything and narrowed afterwards leaks the first time
        // somebody adds a page that forgets to narrow.
        let projects = access::projects_for(&self.state.database, &account).await?;

        layout::head("Projects");
        let frame = Frame::new(&account, Area::Projects, &projects, None, "/");
        let body = rsx! {
                (layout::style_tag())
                <div class="split">
                    <h1>("Projects")</h1>
                    <a class="btn" href="/projects/new">("Create project")</a>
                </div>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }

                @if projects.is_empty() {
                    // No second "Create project" here: the one in the
                    // header is on this page whether or not the list is
                    // empty, and two buttons a hand's width apart that
                    // do the same thing read as two different things.
                    <section class="empty">
                        <p>("No projects yet.")</p>
                    </section>
                } @else {
                    <div class="grid">
                        @for project in &projects {
                            <a class="card tile" href=(format!("/projects/{}", project.slug))>
                                <p class="tile-name">(&project.name)</p>
                                <p class="tile-detail">(&project.slug)</p>
                            </a>
                        }
                    </div>
                }
        }
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }

    #[view("/projects/new")]
    #[middleware(SessionMiddleware)]
    async fn new_project(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };

        let projects = access::projects_for(&self.state.database, &account).await?;

        layout::head("Create project");
        let frame = Frame::new(&account, Area::Projects, &projects, None, "/projects/new");
        let body = rsx! {
                (layout::style_tag())
                <h1>("Create project")</h1>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }
                <form method="post" action="/projects" class="card stack">
                    <label for="name">("Name")</label>
                    <input id="name" name="name" type="text" required autofocus>
                    <p class="field-hint">(
                        "The slug is derived from the name, and it is what \
                         hostnames and containerd labels are built from."
                    )</p>

                    <div class="actions">
                        <button type="submit">("Create project")</button>
                        <a class="btn btn-ghost" href="/">("Cancel")</a>
                    </div>
                </form>
        }
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }

    /// One project: its services, and what each of them is doing.
    #[view("/projects/:project")]
    #[middleware(SessionMiddleware)]
    async fn project(&self, query: ProjectQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };

        // "Not yours" answers exactly like "not there". Telling them
        // apart turns the project list into something anybody can
        // enumerate by guessing names.
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let services = services::all(&self.state.database, Some(&project.id)).await?;
        let new_service = format!("/projects/{}/services/new", project.slug);

        // Asked of containerd, one service at a time. Sequential
        // because a project has a handful of services and each answer
        // is a round trip to a local socket — a fan-out here would buy
        // milliseconds and cost the ability to read the code.
        // One question for the whole page rather than one per row: the
        // answer is a set, and asking the job store per service would
        // be the same read repeated.
        let deploying = crate::deploy::jobs::deploying(&self.state.container).await;

        let mut rows = Vec::with_capacity(services.len());
        for service in services {
            let observed = self
                .state
                .deployer
                .observe_service(&project, &service)
                .await;
            // The address and the reason live on the copy that runs
            // here — a service is *n* of them now, and this row still
            // shows one.
            let placements =
                crate::platform::replicas::of_service(&self.state.database, &service.id).await?;
            let here = placements
                .iter()
                .find(|replica| replica.is_here() && !replica.evicted())
                .cloned();
            let where_it_runs = where_it_runs(&placements);
            // The same value the stream sends, so the first paint and
            // every update after it cannot disagree about which action
            // applies or what the badge says.
            let cell = state_cell(
                &observed,
                deploying.contains(&service.id),
                Some(where_it_runs.as_str()),
            );
            rows.push((service, cell, here.and_then(|replica| replica.last_error)));
        }

        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let path = format!("/projects/{}", project.slug);

        layout::head(&project.name);
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            path,
        )
        .allowing(allowed);
        let body = rsx! {
                (layout::style_tag())
                <div class="split">
                    <div class="stack-sm">
                        <h1>(&project.name)</h1>
                        <p class="slug-preview">(&project.slug)</p>
                    </div>
                    @if allowed.may_deploy() {
                        <a class="btn" href=(&new_service)>("Create service")</a>
                    }
                </div>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }

                @if rows.is_empty() {
                    <section class="empty">
                        <p>("No services yet.")</p>
                        @if allowed.may_deploy() {
                            <a class="btn" href=(&new_service)>("Create service")</a>
                        }
                    </section>
                } @else {
                    (service_table(&project, &rows, allowed))
                }

        }
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }

    /// Who is in this project, and what they may do.
    ///
    /// Its own page rather than a card under the services, because the
    /// two are read at different times by different people: the list of
    /// services is what somebody opens twenty times a day, and access
    /// is what somebody changes when a person joins or leaves.
    ///
    /// Readable by anyone who can see the project — knowing who else is
    /// here is not privileged — while the forms need administration.
    #[view("/projects/:project/people")]
    #[middleware(SessionMiddleware)]
    async fn project_people(&self, query: ProjectQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };

        let members = access::members(&self.state.database, &project.id).await?;
        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let here = format!("/projects/{}", project.slug);
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("{here}/people"),
        )
        .allowing(allowed);

        layout::head(&format!("{} people", project.name));
        let body = rsx! {
                (layout::style_tag())
                <div class="split">
                    <div class="stack-sm">
                        <h1>("People")</h1>
                        <p class="slug-preview">(&project.slug)</p>
                    </div>
                    <span class="who">("You are: ")(allowed.label())</span>
                </div>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }

                <table>
                    <tbody>
                        @for member in &members {
                            <tr>
                                <td>(&member.username)</td>
                                <td>(member.role.label())</td>
                                <td>
                                    @if allowed.may_administer() {
                                        <form method="post"
                                              action=(format!(
                                                  "{here}/people/{}/remove", member.account_id
                                              ))>
                                            <button class="btn btn-ghost destructive btn-sm"
                                                    type="submit">
                                                ("Remove")
                                            </button>
                                        </form>
                                    }
                                </td>
                            </tr>
                        }
                        @if members.is_empty() {
                            <tr>
                                <td class="tile-detail" colspan="3">(
                                    "Nobody but administrators, who reach every project."
                                )</td>
                            </tr>
                        }
                    </tbody>
                </table>

                @if allowed.may_administer() {
                    <form method="post" action=(format!("{here}/people")) class="card stack">
                        <label for="username">("Username")</label>
                        <div class="row">
                            <input id="username" name="username" type="text"
                                   placeholder="username" required>
                            <select name="role">
                                @for role in ProjectRole::ALL {
                                    <option value=(role.as_str())>(role.label())</option>
                                }
                            </select>
                            <button type="submit">("Add")</button>
                        </div>
                        <p class="field-hint">(
                            "Somebody who already has an account on this node. To bring \
                             in a new person, invite them from the people page."
                        )</p>
                    </form>
                }
        }
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }

    /// What the project is configured with, and how to destroy it.
    ///
    /// Push tokens and the danger zone: the two things nobody opens a
    /// project page *for*, and the two that were hardest to find under
    /// a table of services.
    #[view("/projects/:project/settings")]
    #[middleware(SessionMiddleware)]
    async fn project_settings(&self, query: ProjectQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let here = format!("/projects/{}", project.slug);
        // A viewer may look at this project and change nothing on this
        // page, so it has nothing for them. The checks that count are
        // still on each POST.
        if !allowed.may_deploy() {
            return Ok(Redirect::found(here).into());
        }

        let tokens = crate::platform::tokens::of_project(&self.state.database, &project.id).await?;
        // Spent on read. "Shown once" used to mean "in the URL until
        // you navigate away", so a refresh put it back on the screen
        // and the back button found it an hour later.
        let revealed = query
            .shown
            .as_deref()
            .and_then(|nonce| self.state.reveals.take(nonce));
        let registry_host = self
            .state
            .config
            .node
            .domain
            .clone()
            .unwrap_or_else(|| "this node".into());

        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("{here}/settings"),
        )
        .allowing(allowed);

        layout::head(&format!("{} settings", project.name));
        let body = rsx! {
                (layout::style_tag())
                <div class="split">
                    <div class="stack-sm">
                        <h1>("Settings")</h1>
                        <p class="slug-preview">(&project.slug)</p>
                    </div>
                    <a class="btn btn-ghost" href=(&here)>("Back to project")</a>
                </div>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }

                <section class="card stack">
                    <div class="split">
                        <p class="card-label">("Push tokens")</p>
                        <span class="who">(&registry_host)("/")(&project.slug)("/…")</span>
                    </div>
                    @if let Some(secret) = &revealed {
                        <p class="field-hint">(
                            "Shown once. Use it as the password: "
                        )</p>
                        <pre><code>("docker login ")(&registry_host)(" -u ci -p ")(secret)</code></pre>
                    }
                    @if tokens.is_empty() {
                        <p class="tile-detail">(
                            "None. A token is what CI authenticates with — it is nobody's \
                             password, and revoking it changes nothing else."
                        )</p>
                    } @else {
                        <table>
                            <tbody>
                                @for token in &tokens {
                                    <tr>
                                        <td>(&token.name)</td>
                                        <td class="tile-detail">
                                            @if token.last_used_at.is_some() {
                                                ("used")
                                            } @else {
                                                ("never used")
                                            }
                                        </td>
                                        <td>
                                            <form method="post"
                                                  action=(format!(
                                                      "{here}/tokens/{}/revoke", token.id
                                                  ))>
                                                <button class="btn btn-ghost destructive btn-sm"
                                                        type="submit">("Revoke")</button>
                                            </form>
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    }
                    <form method="post" action=(format!("{here}/tokens")) class="row">
                        <input name="name" type="text" placeholder="what it is for" required>
                        <button class="btn btn-secondary" type="submit">("Create token")</button>
                    </form>
                </section>

                @if allowed.may_administer() {
                    <section class="card stack">
                        <p class="card-label">("Danger zone")</p>
                        <p class="tile-detail">(
                            "Deleting a project deletes every service under it. \
                             Nothing is stopped first — do that yourself."
                        )</p>
                        <form method="post" action=(format!("{here}/delete"))>
                            <button class="btn btn-danger" type="submit">("Delete project")</button>
                        </form>
                    </section>
                }
        }
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }
}

impl ProjectApi {
    /// The project this request is about, what the caller may do with
    /// it, and the path split up.
    ///
    /// `None` when there is no session, no such project, or it is not
    /// theirs — the three cases a handler answers identically, which is
    /// what keeps "not yours" from being distinguishable from "not
    /// there".
    #[allow(clippy::type_complexity)]
    async fn locate<'a>(
        &self,
        path: &'a str,
    ) -> RestResult<
        Option<(
            crate::platform::projects::Project,
            crate::accounts::roles::Access,
            Vec<&'a str>,
        )>,
    > {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(None);
        };
        let segments = super::auth::segments(path);
        let Some(slug) = segments.get(1) else {
            return Ok(None);
        };
        Ok(access::find_project(&self.state.database, &account, slug)
            .await?
            .map(|(project, allowed)| (project, allowed, segments)))
    }
}

/// What a service is doing, from containerd rather than from a column.
///
/// The distinction the badge exists to make: "Running" is a task with a
/// pid, and everything else says which kind of not-running it is. A
/// runtime that cannot be reached is its own answer, because
/// redeploying is the wrong response to it.
/// The services table, as an island host.
///
/// Its own function for the reason `Frame::render` gives: an `rsx!`
/// expands to a closure that captures by move, so nesting one inside
/// the page's would have both wanting the same `project` and `rows`.
/// A reference moves freely — it is `Copy` — and the markup is
/// rendered before the host wraps it.
fn service_table(
    project: &crate::platform::projects::Project,
    // The third is why the copy running here is not, when it is not.
    // Carried alongside rather than read off the service: it belongs to
    // a replica now, and a service is several of them.
    rows: &[(services::Service, StateCell, Option<String>)],
    allowed: crate::accounts::roles::Access,
) -> impl Renderable {
    let inner = rsx! {
                    <table>
                        <thead>
                            <tr>
                                <th>("Service")</th>
                                <th class="address">("Address")</th>
                                <th class="state">("State")</th>
                                // The delete column. Headed by nothing,
                                // because "Delete" over a column of
                                // Delete buttons reads as a heading for
                                // a thing rather than a label for one.
                                <th></th>
                            </tr>
                        </thead>
                        <tbody>
                            @for (service, cell, failure) in rows {
                                <tr>
                                    <td>
                                        <a href=(format!(
                                            "/projects/{}/services/{}",
                                            project.slug, service.slug
                                        ))>(&service.name)</a>
                                    </td>
                                    <td class="mono address" data-address=(&service.id)>
                                        (&cell.address)
                                    </td>
                                    <td class="state" data-state=(&service.id)>
                                        (state_badge(cell))
                                    </td>
                                    <td class="row">
                                        @if allowed.may_deploy() {
                                            (deploy_controls(
                                                &project.slug, &service.slug, cell
                                            ))
                                        }
                                    </td>
                                </tr>
                                @if let Some(failure) = failure {
                                    <tr>
                                        <td colspan="4" class="failure">(failure)</td>
                                    </tr>
                                }
                            }
                        </tbody>
                    </table>
    }
    .render()
    .into_inner();

    wabot::ui::hypertext::island(
        "project-live",
        &serde_json::json!({ "project": &project.slug }),
        hypertext::Raw::dangerously_create(&inner),
    )
}

/// Whether a row's control is the one that applies.
///
/// A class rather than the `hidden` attribute, because `hidden` is
/// boolean *by presence*: `rsx!` renders `hidden=(false)` as
/// `hidden="false"`, which hides the element just as thoroughly. Both
/// controls were hidden on first paint and only appeared when the
/// stream's first message set the DOM property. `class` has no such
/// trap — an empty one matches nothing.
fn shown(applies: bool) -> &'static str {
    if applies {
        ""
    } else {
        "is-hidden"
    }
}

/// One control, disabled or not.
///
/// Written twice rather than `disabled=(busy)`, for the same reason:
/// `disabled="false"` disables. There is no way to spell "no attribute"
/// in an `rsx!` attribute value, so the branch is on the element.
/// The play and the stop, for whichever page is showing a service.
///
/// One definition, because the project's list and the service's own
/// page must not disagree about which control applies — and they had
/// no way not to, since the detail page simply had neither. A service
/// you can see the state of and cannot start is a page that shows a
/// problem and withholds the fix.
///
/// Both actions are rendered and one is hidden, so the stream can swap
/// them by toggling: an island may hide what does not apply, not build
/// what does. The check that counts is on the POST; this is so the page
/// does not offer what it will refuse.
pub(crate) fn deploy_controls<'a>(
    project_slug: &'a str,
    service_slug: &'a str,
    cell: &'a StateCell,
) -> impl Renderable + 'a {
    rsx! {
        <form method="post" data-action="deploy"
              class=(shown(cell.action == "deploy"))
              action=(format!("/projects/{project_slug}/services/{service_slug}/deploy"))>
            (action_button("Deploy", PLAY, cell.busy))
        </form>
        <form method="post" data-action="stop"
              class=(shown(cell.action == "stop"))
              action=(format!("/projects/{project_slug}/services/{service_slug}/stop"))>
            (action_button("Stop", STOP, cell.busy))
        </form>
    }
}

fn action_button(label: &'static str, icon: &'static str, busy: bool) -> impl Renderable {
    rsx! {
        @if busy {
            <button class="btn btn-secondary btn-sm icon" type="submit"
                    title=(label) aria-label=(label) disabled>
                (hypertext::Raw::dangerously_create(icon))
            </button>
        } @else {
            <button class="btn btn-secondary btn-sm icon" type="submit"
                    title=(label) aria-label=(label)>
                (hypertext::Raw::dangerously_create(icon))
            </button>
        }
    }
}

/// Lucide `play` and `square`, inlined.
///
/// Inlined rather than fetched: the design system names Lucide for the
/// rare icon, and this console cannot reach a CDN — the same reason the
/// fonts are vendored. A deliberate substitution, since the original
/// product has no icon here.
///
/// Raw because `rsx!` validates element names and does not know SVG's.
/// XSS SAFETY: two constants in this file, never a value from a
/// request.
const PLAY: &str = r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polygon points="6 3 20 12 6 21 6 3"/></svg>"#;

const STOP: &str = r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>"#;

/// One service's state, as the three things the badge shows.
///
/// Formatted here so the first paint and every update after it come
/// from one place. The stream only assigns text and classes that the
/// server already rendered — see the island rules in CLAUDE.md.
#[derive(serde::Serialize)]
pub(crate) struct StateCell {
    word: String,
    badge: &'static str,
    dot: &'static str,
    /// Which control applies: `deploy` or `stop`. Always one of the
    /// two — a control that vanishes takes the column's width with it
    /// and leaves nothing to press or to read.
    action: &'static str,
    /// Whether it is pressable. A deployment in flight shows `stop`
    /// disabled: that is where it is heading, and saying so is more
    /// use than an empty cell.
    busy: bool,
    /// Where it runs, in the width of a column.
    ///
    /// The container's address while a service is one copy on this
    /// node, which is what it usually is and the most useful thing to
    /// show. Once there are several, one address is a lie about the
    /// other n − 1, so it becomes a count instead — the service's own
    /// page is where each copy is named.
    address: String,
}

/// Where a service runs, in the width of a column.
///
/// One copy here is the ordinary case and its address is the useful
/// thing. Several copies make one address a lie about the others, so
/// they become a count — and a copy somewhere else is worth saying
/// even when there is only one, because "running" on this page would
/// otherwise mean a container this node has never seen.
pub(crate) fn where_it_runs(placements: &[crate::platform::replicas::Replica]) -> String {
    let live: Vec<&crate::platform::replicas::Replica> =
        placements.iter().filter(|r| !r.evicted()).collect();
    let elsewhere = live.iter().filter(|r| !r.is_here()).count();

    match (live.len(), elsewhere) {
        (1, 0) => live[0].address.clone().unwrap_or_else(|| "—".into()),
        (1, 1) => "1 elsewhere".to_string(),
        (n, 0) => format!("{n} replicas"),
        (n, away) => format!("{n} replicas · {away} elsewhere"),
    }
}

pub(crate) fn state_cell(observed: &Observed, deploying: bool, address: Option<&str>) -> StateCell {
    let address = address.unwrap_or("—").to_string();
    if deploying {
        return StateCell {
            word: "Deploying".into(),
            badge: "badge badge-info",
            dot: "dot dot-info dot-pulse",
            action: "stop",
            busy: true,
            address,
        };
    }
    match observed {
        Observed::Running { .. } => StateCell {
            word: "Running".into(),
            badge: "badge badge-success",
            dot: "dot dot-success",
            action: "stop",
            busy: false,
            address,
        },
        Observed::Stopped { exit_code } => StateCell {
            word: format!("Exited {exit_code}"),
            badge: "badge badge-danger",
            dot: "dot dot-danger",
            action: "deploy",
            busy: false,
            address: address.clone(),
        },
        Observed::Absent => StateCell {
            word: "Not deployed".into(),
            badge: "badge badge-warning",
            dot: "dot dot-warning",
            action: "deploy",
            busy: false,
            address: address.clone(),
        },
        Observed::Unknown(_) => StateCell {
            word: "Unknown".into(),
            badge: "badge badge-info",
            dot: "dot dot-info",
            action: "deploy",
            busy: false,
            address: address.clone(),
        },
    }
}

/// What a service is doing, as a dot and a word.
///
/// Rendered from the same [`StateCell`] the stream sends, so the first
/// paint and every update after it come from one place. They used to be
/// two functions that had to be kept in step, which is the kind of pair
/// that drifts.
pub(crate) fn state_badge(cell: &StateCell) -> impl Renderable + '_ {
    rsx! {
        <span class=(cell.badge)>
            <span class=(cell.dot)></span>(&cell.word)
        </span>
    }
}

#[injectable]
pub struct ProjectApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl ProjectApi {
    #[post("/projects")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        // The POST is the boundary, not the page: a form can be
        // submitted by anything, and the page that renders it is only
        // the polite half of this check.
        if signed_in(&self.auth).is_none() {
            return Ok(see_other("/sign-in"));
        }

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let name = field(&form, "name");

        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };

        match projects::create(&self.state.database, name).await {
            Ok(project) => {
                // Whoever made it owns it. Without this a member would
                // create a project and immediately not be able to see
                // it, since the list is filtered by membership.
                access::grant(
                    &self.state.database,
                    &account.id,
                    &project.id,
                    ProjectRole::Owner,
                )
                .await?;
                Ok(see_other(&format!("/projects/{}", project.slug)))
            }
            Err(error) => Ok(back_with_error("/projects/new", &error.to_string())),
        }
    }

    /// Put somebody who already has an account into this project.
    /// What each service is doing, as server-sent events.
    ///
    /// A tick rather than a signal: a deployment's progress lives in
    /// the job store and containerd, and neither announces itself. Two
    /// seconds is the same cadence the node page uses for memory.
    ///
    /// `#[raw]`, because the body is a stream that never ends and the
    /// JSON path buffers a value.
    #[get("/projects/:project/live")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn live(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some(account) = signed_in(&self.auth) else {
            // A status, not a redirect: an EventSource cannot follow
            // one usefully.
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        };
        let Some(slug) = super::auth::segments(&path).get(1).map(|s| s.to_string()) else {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        };
        // The same check the page makes: a stream is as private as
        // what it streams.
        let Some((project, _)) =
            access::find_project(&self.state.database, &account, &slug).await?
        else {
            return Ok(Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        };

        let state = self.state.clone();
        let stream = async_stream::stream! {
            loop {
                let mut cells = std::collections::BTreeMap::new();
                if let Ok(services) =
                    services::all(&state.database, Some(&project.id)).await
                {
                    let deploying =
                        crate::deploy::jobs::deploying(&state.container).await;
                    for service in services {
                        let observed = state.deployer.observe_service(&project, &service).await;
                        let busy = deploying.contains(&service.id);
                        cells.insert(
                            service.id.clone(),
                            state_cell(
                                &observed,
                                busy,
                                Some(
                                    where_it_runs(
                                        &crate::platform::replicas::of_service(
                                            &state.database,
                                            &service.id,
                                        )
                                        .await
                                        .unwrap_or_default(),
                                    )
                                    .as_str(),
                                ),
                            ),
                        );
                    }
                }
                let payload = serde_json::to_string(&cells)
                    .unwrap_or_else(|_| "{}".into());
                yield Ok::<_, std::convert::Infallible>(
                    wabot::rest::axum::body::Bytes::from(format!("data: {payload}\n\n")),
                );
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        };

        Ok(Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            // Every hop has to be told, or a proxy holds the stream
            // until it has "enough" and the page never updates.
            .header(header::CACHE_CONTROL, "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(stream))
            .expect("a constant response is well-formed"))
    }

    #[post("/projects/:project/people")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add_member(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+project"));
        };
        // Back to the page the form is on. It used to be the project
        // overview, which is where these forms lived.
        let here = format!("/projects/{}/people", project.slug);
        if !allowed.may_administer() {
            return Ok(back_with_error(
                &here,
                "only an owner can change who is here",
            ));
        }

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let username = field(&form, "username");
        let role = ProjectRole::parse(field(&form, "role"));

        let Some(found) = crate::accounts::all(&self.state.database)
            .await?
            .into_iter()
            .find(|candidate| candidate.username.eq_ignore_ascii_case(username))
        else {
            return Ok(back_with_error(
                &here,
                &format!("nobody on this node is called {username:?} — invite them first"),
            ));
        };

        access::grant(&self.state.database, &found.id, &project.id, role).await?;
        Ok(see_other(&here))
    }

    /// Take somebody out of this project.
    #[post("/projects/:project/people/:account/remove")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn remove_member(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, segments)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+project"));
        };
        // Back to the page the form is on. It used to be the project
        // overview, which is where these forms lived.
        let here = format!("/projects/{}/people", project.slug);
        if !allowed.may_administer() {
            return Ok(back_with_error(
                &here,
                "only an owner can change who is here",
            ));
        }

        let Some(account_id) = segments.get(3) else {
            return Ok(see_other(&here));
        };
        match access::revoke(&self.state.database, account_id, &project.id).await {
            Ok(()) => Ok(see_other(&here)),
            Err(error) => Ok(back_with_error(&here, &error.to_string())),
        }
    }

    /// The side nav's project selector.
    ///
    /// A POST because it is a form, and a redirect because what the
    /// operator asked for is to *be* on that project's page. There is
    /// no stored selection: the URL is the selection, which is what
    /// makes a link to a project mean the same thing for everyone.
    #[post("/select-project")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn select(&self, request: Request) -> RestResult<Response> {
        if signed_in(&self.auth).is_none() {
            return Ok(see_other("/sign-in"));
        }

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let slug = field(&form, "project");

        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        match access::find_project(&self.state.database, &account, slug).await? {
            Some((project, _)) => Ok(see_other(&format!("/projects/{}", project.slug))),
            None => Ok(see_other("/?error=no+such+project")),
        }
    }

    /// Mint a push token for this project.
    #[post("/projects/:project/tokens")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create_token(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+project"));
        };
        // Back to the page the form is on. It used to be the project
        // overview, which is where these forms lived.
        let here = format!("/projects/{}/settings", project.slug);
        if !allowed.may_deploy() {
            return Ok(back_with_error(
                &here,
                "you may look at this project, not push to it",
            ));
        }
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::platform::tokens::create(
            &self.state.database,
            &project.id,
            field(&form, "name"),
            &account.id,
        )
        .await
        {
            // The nonce travels, not the secret. This is the only
            // moment the token exists in clear — it is stored hashed,
            // so no page can ever show it again — and a query string is
            // read by the address bar, the history and every refresh.
            // See `ConsoleState::reveals`.
            Ok((_, secret)) => {
                let nonce = self.state.reveals.stash(secret);
                Ok(see_other(&format!(
                    "{here}?{}",
                    form_urlencoded::Serializer::new(String::new())
                        .append_pair("shown", &nonce)
                        .finish()
                )))
            }
            Err(error) => Ok(back_with_error(&here, &error.to_string())),
        }
    }

    #[post("/projects/:project/tokens/:token/revoke")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revoke_token(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, segments)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+project"));
        };
        // Back to the page the form is on. It used to be the project
        // overview, which is where these forms lived.
        let here = format!("/projects/{}/settings", project.slug);
        if !allowed.may_deploy() {
            return Ok(see_other(&here));
        }

        // Only this project's tokens: an id from another page must not
        // revoke somebody else's.
        if let Some(id) = segments.get(3) {
            let owned = crate::platform::tokens::of_project(&self.state.database, &project.id)
                .await?
                .into_iter()
                .any(|token| token.id == *id);
            if owned {
                crate::platform::tokens::revoke(&self.state.database, id).await?;
            }
        }
        Ok(see_other(&here))
    }

    /// Delete a project and, by cascade, its services.
    ///
    /// A POST, so nothing a browser prefetches can trigger it, and no
    /// confirmation dialog: a dialog needs JavaScript, and this console
    /// works without it. The button lives under a "Danger zone"
    /// heading, which is the warning.
    #[post("/projects/:project/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        if signed_in(&self.auth).is_none() {
            return Ok(see_other("/sign-in"));
        }

        let path = request.uri().path().to_string();
        let Some((project, allowed, _)) = self.locate(&path).await? else {
            // Already gone, or never theirs. Both answer the same.
            return Ok(see_other("/"));
        };
        if !allowed.may_administer() {
            return Ok(back_with_error(
                &format!("/projects/{}", project.slug),
                "only an owner can delete this project",
            ));
        }

        projects::delete(&self.state.database, &project.id).await?;
        Ok(see_other("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    /// The property the whole model rests on: a project you are not in
    /// is one you cannot see, cannot open, and cannot change.
    #[tokio::test]
    async fn a_member_cannot_reach_a_project_they_are_not_in() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &admin)
            .form(&[("name", "not theirs")])
            .send()
            .await;

        let member = console.joined_as(&admin, "member").await;

        let list = console
            .harness
            .get("/")
            .header("cookie", &member)
            .send()
            .await
            .body;
        assert!(
            list.contains("No projects yet"),
            "the list is empty:\n{list}"
        );

        // The page answers exactly as it would for a project that does
        // not exist.
        let page = console
            .harness
            .get("/projects/not-theirs")
            .header("cookie", &member)
            .send()
            .await;
        assert_eq!(page.header("location"), Some("/?error=no+such+project"));

        // And so does deleting it.
        console
            .harness
            .post("/projects/not-theirs/delete")
            .header("cookie", &member)
            .send()
            .await;
        assert_eq!(
            projects::all(&console.database).await.expect("query").len(),
            1,
            "it is still there"
        );
    }

    /// Whoever makes a project owns it — otherwise a member would
    /// A push token used to ride to its page in the query string. That
    /// put the one secret this console ever shows into the address bar,
    /// the browser's history, and back on the screen on every refresh.
    /// The redirect carries a nonce now, and reading spends it.
    #[tokio::test]
    async fn a_new_token_is_shown_once_and_never_in_the_url() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &admin)
            .form(&[("name", "shipping")])
            .send()
            .await;

        let response = console
            .harness
            .post("/projects/shipping/tokens")
            .header("cookie", &admin)
            .form(&[("name", "ci")])
            .send()
            .await;
        let location = response.header("location").expect("redirected").to_string();
        assert!(location.contains("shown="), "{location}");

        let nonce = location
            .split("shown=")
            .nth(1)
            .expect("a nonce")
            .to_string();
        let first = console
            .harness
            .get(&location)
            .header("cookie", &admin)
            .send()
            .await
            .body;
        assert!(first.contains("docker login"), "shown once: {first}");
        assert!(
            !first.contains(&nonce),
            "what travelled is not what was shown"
        );

        let again = console
            .harness
            .get(&location)
            .header("cookie", &admin)
            .send()
            .await
            .body;
        assert!(
            !again.contains("docker login"),
            "and a refresh does not show it again: {again}"
        );
    }

    /// create one and immediately not be able to see it.
    #[tokio::test]
    async fn creating_a_project_makes_you_its_owner() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        console
            .harness
            .post("/projects")
            .header("cookie", &member)
            .form(&[("name", "mine")])
            .send()
            .await;

        let body = console
            .harness
            .get("/projects/mine/settings")
            .header("cookie", &member)
            .send()
            .await
            .body;
        assert!(body.contains("Danger zone"), "an owner sees it: {body}");
    }

    /// One address is a lie about the other n − 1. The column says
    /// where a service runs, and only names an address while there is
    /// exactly one copy to name.
    #[test]
    fn the_column_stops_naming_an_address_once_there_are_several() {
        let replica =
            |slot, here: bool, address: Option<&str>| crate::platform::replicas::Replica {
                id: format!("rp-{slot}"),
                service_id: "svc-1".into(),
                node_id: (!here).then(|| "nd-elsewhere".to_string()),
                slot,
                address: address.map(str::to_string),
                last_error: None,
                evicted_at: None,
            };

        // The ordinary case, and the useful one.
        assert_eq!(
            where_it_runs(&[replica(1, true, Some("10.42.1.5"))]),
            "10.42.1.5"
        );
        // One copy and it is not on this machine: an address here would
        // name a container this node has never seen.
        assert_eq!(where_it_runs(&[replica(1, false, None)]), "1 elsewhere");

        assert_eq!(
            where_it_runs(&[replica(1, true, Some("10.42.1.5")), replica(2, true, None)]),
            "2 replicas"
        );
        assert_eq!(
            where_it_runs(&[
                replica(1, true, Some("10.42.1.5")),
                replica(2, false, None),
                replica(3, false, None),
            ]),
            "3 replicas · 2 elsewhere"
        );
    }

    /// During a deployment containerd's answer is a half-truth — the
    /// old container is gone and the new one is not up — so "absent"
    /// there reads as a fault. The job is what knows.
    #[test]
    fn a_deployment_in_flight_outranks_what_containerd_says() {
        let busy = state_cell(&Observed::Absent, true, None);
        assert_eq!(busy.word, "Deploying");
        // Shown, not hidden: a control that vanishes takes the column's
        // width with it. Disabled says the same thing and keeps the row
        // still — and it names where the deployment is heading.
        assert_eq!(busy.action, "stop");
        assert!(busy.busy, "and it cannot be pressed yet");

        let idle = state_cell(&Observed::Absent, false, None);
        assert_eq!(idle.word, "Not deployed");
        assert_eq!(idle.action, "deploy");
        assert!(!idle.busy);

        // Running offers Stop, which is the pairing the row used to get
        // wrong the moment the badge updated and the button did not.
        let running = state_cell(
            &Observed::Running {
                pid: 1,
                address: None,
            },
            false,
            Some("10.42.1.5"),
        );
        assert_eq!(running.action, "stop");
        // The address rides with the state: a deployment ends by
        // assigning one, and a row showing the new state beside the old
        // address is half-updated.
        assert_eq!(running.address, "10.42.1.5");
    }

    /// The state updates in place, so the page has to declare the
    /// island and a hook per service — a stream with nothing to write
    /// into is a connection held open for nothing.
    #[tokio::test]
    async fn the_services_table_is_a_live_island() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "shipping")])
            .send()
            .await;
        console
            .harness
            .post("/projects/shipping/services")
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;

        let page = console
            .ui
            .with_header("cookie", cookie.clone())
            .get("/projects/shipping")
            .await;
        assert!(page.has_island("project-live"), "{}", page.html());
        assert_eq!(
            page.island_props("project-live"),
            Some(serde_json::json!({ "project": "shipping" })),
            "the client needs the slug to open the stream"
        );

        let service = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        assert!(
            page.html()
                .contains(&format!("data-state=\"{}\"", service.id)),
            "and a hook to write into: {}",
            page.html()
        );
    }

    /// The stream is as private as the table it feeds.
    #[tokio::test]
    async fn the_project_stream_needs_a_session() {
        let console = Console::new().await;
        console.signed_in().await;

        let response = console.harness.get("/projects/shipping/live").send().await;
        response.assert_status(StatusCode::UNAUTHORIZED);
    }

    /// `hidden` and `disabled` are boolean *by presence*: rendering
    /// `hidden="false"` hides just as thoroughly. Both controls were
    /// hidden on first paint and only appeared once the stream's first
    /// message set the DOM property, so a reload showed a row with
    /// nothing to press for a second.
    #[tokio::test]
    async fn a_control_that_applies_is_visible_before_any_script_runs() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "shipping")])
            .send()
            .await;
        console
            .harness
            .post("/projects/shipping/services")
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;

        let body = console
            .harness
            .get("/projects/shipping")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            !body.contains("hidden=\"false\"") && !body.contains("disabled=\"false\""),
            "an attribute the browser reads as true: {body}"
        );
        // Which control applies depends on whether the deployment this
        // page just queued has finished, which is a race — CI lost it
        // where this machine won. The invariant is the point: exactly
        // one of the two is hidden, so the row always has something to
        // press and never has two.
        assert_eq!(
            body.matches(r#"class="is-hidden""#).count(),
            1,
            "exactly one control is hidden: {body}"
        );
    }

    /// One page, one subject. The overview used to carry the services,
    /// the push tokens, the people and the delete button — four things
    /// somebody came for at four different moments, and the one they
    /// came for most often was the one they had to scroll past.
    #[tokio::test]
    async fn the_overview_carries_services_and_nothing_else() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "shipping")])
            .send()
            .await;

        let get = |path: &'static str| {
            let harness = &console.harness;
            let cookie = cookie.clone();
            async move {
                harness
                    .get(path)
                    .header("cookie", &cookie)
                    .send()
                    .await
                    .body
            }
        };

        let overview = get("/projects/shipping").await;
        for elsewhere in ["Push tokens", "Danger zone", "Create token"] {
            assert!(
                !overview.contains(elsewhere),
                "{elsewhere} is still on the overview: {overview}"
            );
        }
        assert!(overview.contains("No services yet"), "{overview}");

        // Not empty: creating a project grants the creator Owner, so
        // there is always at least one person to list.
        let people = get("/projects/shipping/people").await;
        assert!(people.contains("Owner"), "{people}");
        assert!(people.contains("Username"), "and a way to add one");

        let settings = get("/projects/shipping/settings").await;
        assert!(settings.contains("Push tokens"), "{settings}");
        assert!(settings.contains("Danger zone"), "{settings}");
    }

    /// Each form comes back to the page it is on, or the next edit
    /// starts with a click nobody should have had to make.
    #[tokio::test]
    async fn a_project_form_returns_to_its_own_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "shipping")])
            .send()
            .await;

        for (action, form, expected) in [
            (
                "tokens",
                vec![("name", "ci")],
                "/projects/shipping/settings",
            ),
            (
                "people",
                vec![("username", "nobody"), ("role", "viewer")],
                "/projects/shipping/people",
            ),
        ] {
            let response = console
                .harness
                .post(&format!("/projects/shipping/{action}"))
                .header("cookie", &cookie)
                .form(&form)
                .send()
                .await;
            let location = response.header("location").expect("redirected").to_string();
            assert!(
                location.starts_with(expected),
                "{action} went to {location}"
            );
        }
    }

    /// A viewer sees the state and is offered nothing to press — and
    /// the POST behind each button refuses them anyway, which is the
    /// check that counts.
    #[tokio::test]
    async fn a_viewer_is_offered_nothing_and_refused_everything() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &admin)
            .form(&[("name", "shared")])
            .send()
            .await;
        let member = console.joined_as(&admin, "watcher").await;

        let watcher = crate::accounts::all(&console.database)
            .await
            .expect("query")
            .into_iter()
            .find(|account| account.username == "watcher")
            .expect("joined");
        let project = projects::find(&console.database, "shared")
            .await
            .expect("query")
            .expect("present");
        access::grant(
            &console.database,
            &watcher.id,
            &project.id,
            ProjectRole::Viewer,
        )
        .await
        .expect("granted");

        let body = console
            .harness
            .get("/projects/shared")
            .header("cookie", &member)
            .send()
            .await
            .body;
        assert!(body.contains("shared"), "they can see it");
        assert!(!body.contains("Create service"), "and press nothing");

        // Settings is nothing but controls they would be refused, so
        // it is not a page for them at all.
        let refused = console
            .harness
            .get("/projects/shared/settings")
            .header("cookie", &member)
            .send()
            .await;
        assert_eq!(
            refused.header("location"),
            Some("/projects/shared"),
            "a viewer was shown the settings page"
        );

        console
            .harness
            .post("/projects/shared/services")
            .header("cookie", &member)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;
        assert!(
            services::all(&console.database, None)
                .await
                .expect("query")
                .is_empty(),
            "a viewer created a service"
        );

        console
            .harness
            .post("/projects/shared/delete")
            .header("cookie", &member)
            .send()
            .await;
        assert_eq!(
            projects::all(&console.database).await.expect("query").len(),
            1,
            "a viewer deleted the project"
        );
    }

    /// A deployer runs things and owns nothing.
    #[tokio::test]
    async fn a_deployer_may_change_services_but_not_the_project() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &admin)
            .form(&[("name", "shared")])
            .send()
            .await;
        let member = console.joined_as(&admin, "deployer").await;

        let person = crate::accounts::all(&console.database)
            .await
            .expect("query")
            .into_iter()
            .find(|account| account.username == "deployer")
            .expect("joined");
        let project = projects::find(&console.database, "shared")
            .await
            .expect("query")
            .expect("present");
        access::grant(
            &console.database,
            &person.id,
            &project.id,
            ProjectRole::Deployer,
        )
        .await
        .expect("granted");

        console
            .harness
            .post("/projects/shared/services")
            .header("cookie", &member)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;
        assert_eq!(
            services::all(&console.database, None)
                .await
                .expect("query")
                .len(),
            1,
            "they may deploy"
        );

        console
            .harness
            .post("/projects/shared/delete")
            .header("cookie", &member)
            .send()
            .await;
        assert_eq!(
            projects::all(&console.database).await.expect("query").len(),
            1,
            "and may not delete the project"
        );
    }

    #[tokio::test]
    async fn the_list_starts_empty_and_says_so() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("No projects yet"), "{body}");
    }

    #[tokio::test]
    async fn creating_a_project_lands_on_its_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "My API")])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/projects/my-api"));

        let body = console
            .harness
            .get("/projects/my-api")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("My API"), "{body}");
        assert!(body.contains("No services yet"), "{body}");
    }

    /// A refused name has to come back with the reason attached, or the
    /// form looks like it silently did nothing.
    #[tokio::test]
    async fn a_refused_name_returns_to_the_form_with_the_reason() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "???")])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").expect("redirected");
        assert!(location.starts_with("/projects/new?error="), "{location}");

        // And the page renders that message rather than dropping it.
        let body = console
            .harness
            .get(location)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("hostname"), "{body}");
    }

    /// A stale link should not be a dead end.
    #[tokio::test]
    async fn an_unknown_project_goes_back_to_the_list() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .get("/projects/never-existed")
            .header("cookie", &cookie)
            .send()
            .await;
        response.assert_status(StatusCode::FOUND);
        assert!(response
            .header("location")
            .is_some_and(|to| to.starts_with("/?error=")));
    }

    #[tokio::test]
    async fn deleting_a_project_takes_its_services_with_it() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "doomed")])
            .send()
            .await;
        console
            .harness
            .post("/projects/doomed/services")
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;

        let response = console
            .harness
            .post("/projects/doomed/delete")
            .header("cookie", &cookie)
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));

        assert!(projects::all(&console.database)
            .await
            .expect("query")
            .is_empty());
        assert!(
            services::all(&console.database, None)
                .await
                .expect("query")
                .is_empty(),
            "the cascade took the services"
        );
    }

    /// Deleting something already gone is the outcome that was asked
    /// for, so it lands on the list rather than on an error.
    #[tokio::test]
    async fn deleting_a_project_twice_is_not_an_error() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "doomed")])
            .send()
            .await;

        for _ in 0..2 {
            let response = console
                .harness
                .post("/projects/doomed/delete")
                .header("cookie", &cookie)
                .send()
                .await;
            assert_eq!(response.header("location"), Some("/"));
        }
    }

    #[tokio::test]
    async fn deleting_a_project_needs_a_session() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "keep me")])
            .send()
            .await;

        let response = console
            .harness
            .post("/projects/keep-me/delete")
            .send()
            .await;
        assert_eq!(response.header("location"), Some("/sign-in"));
        assert_eq!(
            projects::all(&console.database).await.expect("query").len(),
            1,
            "it is still there"
        );
    }

    /// The page and the POST are separate boundaries, and the POST is
    /// the one that matters: a form can be submitted by anything.
    #[tokio::test]
    async fn creating_a_project_needs_a_session() {
        let console = Console::new().await;
        // An account exists, so this is a signed-out visitor rather
        // than an unconfigured node.
        console.signed_in().await;

        let response = console
            .harness
            .post("/projects")
            .form(&[("name", "sneaky")])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/sign-in"));

        assert!(
            projects::all(&console.database)
                .await
                .expect("query")
                .is_empty(),
            "nothing was created"
        );
    }
}
