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

use super::language::t;
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

        // What needs somebody, read once for the page they land on. An
        // administrator's view: a member of one project has no business
        // being told about another's certificates, and every source here
        // is node-wide.
        let concerns = match account.is_admin() {
            true => super::attention::gather(&self.state)
                .await
                .unwrap_or_default(),
            false => Vec::new(),
        };

        layout::head("Projects");
        let frame = Frame::new(&account, Area::Projects, &projects, None, "/");
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let body = super::language::scoped(account.language, || {
            rsx! {
                    (layout::style_tag())
                    <div class="split">
                        <h1>(t("Projects"))</h1>
                        <a class="btn" href="/projects/new">(t("Create project"))</a>
                    </div>
                    @if let Some(message) = &query.error {
                        (layout::error_note(message))
                    }

                    // Above the list, and only when there is something.
                    // A panel that is always there is part of the
                    // wallpaper by the second week.
                    (super::attention::card(&concerns))

                    @if projects.is_empty() {
                        // No second "Create project" here: the one in the
                        // header is on this page whether or not the list is
                        // empty, and two buttons a hand's width apart that
                        // do the same thing read as two different things.
                        <section class="empty">
                            <p>(t("No projects yet."))</p>
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
            .into_inner()
        });

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
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let body = super::language::scoped(account.language, || {
            rsx! {
                (layout::style_tag())
                <h1>(t("Create project"))</h1>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }
                <form method="post" action="/projects" class="card stack">
                    <label for="name">(t("Name"))</label>
                    <input id="name" name="name" type="text" autocomplete="off" required autofocus>
                    <p class="field-hint">(t("The slug is derived from the name, and it is what \
                         hostnames and containerd labels are built from."))</p>

                    <div class="actions">
                        <button type="submit">(t("Create project"))</button>
                        <a class="btn btn-ghost" href="/">(t("Cancel"))</a>
                    </div>
                </form>
        }
            .render()
            .into_inner()
        });

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
        let new_database = format!("/projects/{}/databases/new", project.slug);

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
                where_it_runs,
                elsewhere_of(&placements),
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
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let body = super::language::scoped(account.language, || {
            rsx! {
                    (layout::style_tag())
                    <div class="split">
                        <div class="stack-sm">
                            <h1>(&project.name)</h1>
                        </div>
                        @if allowed.may_deploy() {
                            <div class="row">
                                // Secondary, and beside rather than
                                // above: a database is a service of a
                                // particular kind, not a second thing
                                // this page is about.
                                <a class="btn btn-secondary" href=(&new_database)>(
                                    t("Create database")
                                )</a>
                                <a class="btn" href=(&new_service)>(t("Create service"))</a>
                            </div>
                        }
                    </div>
                    @if let Some(message) = &query.error {
                        (layout::error_note(message))
                    }

                    @if rows.is_empty() {
                        <section class="empty">
                            <p>(t("No services yet."))</p>
                            @if allowed.may_deploy() {
                                <div class="row">
                                    <a class="btn btn-secondary" href=(&new_database)>(
                                        t("Create database")
                                    )</a>
                                    <a class="btn" href=(&new_service)>(t("Create service"))</a>
                                </div>
                            }
                        </section>
                    } @else {
                        (service_table(&project, &rows, allowed))
                    }

            }
            .render()
            .into_inner()
        });

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
    /// Where the people page went.
    ///
    /// Kept as a redirect rather than removed: it was a nav item for
    /// months, so it is in somebody's history and somebody's bookmark,
    /// and a 404 would tell them the feature was gone when it had only
    /// moved one page over.
    #[view("/projects/:project/people")]
    #[middleware(SessionMiddleware)]
    async fn project_people(&self, query: ProjectQuery) -> UiResult<ViewOutcome> {
        Ok(Redirect::found(format!("/projects/{}/settings", query.project)).into())
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
        // The stored domain, not the file's. The registry names what it
        // receives under `settings::domain`, which prefers the row the
        // console writes — so reading the file here printed a `docker
        // login` for whatever the node was called at install time, for
        // as long as nobody had edited the file to match.
        let registry_host = crate::node::settings::domain(&self.state.database, &self.state.config)
            .await
            .unwrap_or_else(|| "this node".into());

        let members = access::members(&self.state.database, &project.id).await?;
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
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let body = super::language::scoped(account.language, || {
            rsx! {
                (layout::style_tag())
                <div class="split">
                    <div class="stack-sm">
                        <h1>(t("Settings"))</h1>
                    </div>
                    // What this account may do here, said once at the
                    // top. It used to be on the people page and is more
                    // use beside the controls it explains: a button
                    // that is not there reads as missing until
                    // something says why.
                    <span class="who">(t("You are: "))(allowed.label())</span>
                </div>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }

                (people_card(&here, &members, allowed))

                <section class="card stack">
                    <div class="split">
                        <p class="card-label">(t("Push tokens"))</p>
                        <span class="who">(&registry_host)("/")(&project.slug)("/…")</span>
                    </div>
                    @if let Some(secret) = &revealed {
                        <p class="field-hint">(t("Shown once. Use it as the password: "))</p>
                        <pre><code>("docker login ")(&registry_host)(" -u ci -p ")(secret)</code></pre>
                    }
                    @if tokens.is_empty() {
                        <p class="tile-detail">(t("None. A token is what CI authenticates with — it is nobody's \
                             password, and revoking it changes nothing else."))</p>
                    } @else {
                        <table>
                            <tbody>
                                @for token in &tokens {
                                    <tr>
                                        <td>(&token.name)</td>
                                        <td class="tile-detail">
                                            @if token.last_used_at.is_some() {
                                                (t("used"))
                                            } @else {
                                                (t("never used"))
                                            }
                                        </td>
                                        <td>
                                            <form method="post"
                                                  action=(format!(
                                                      "{here}/tokens/{}/revoke", token.id
                                                  ))>
                                                <button class="btn btn-ghost destructive btn-sm"
                                                        type="submit">(t("Revoke"))</button>
                                            </form>
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    }
                    <form method="post" action=(format!("{here}/tokens")) class="row">
                        <input name="name" type="text" autocomplete="off" placeholder="what it is for" required>
                        <button class="btn btn-secondary" type="submit">(t("Create token"))</button>
                    </form>
                </section>

                @if allowed.may_administer() {
                    <section class="card stack">
                        <p class="card-label">(t("Danger zone"))</p>
                        <p class="tile-detail">(t("Deleting a project deletes every service under it. \
                             Nothing is stopped first — do that yourself."))</p>
                        <form method="post" action=(format!("{here}/delete"))>
                            <button class="btn btn-danger" type="submit">(t("Delete project"))</button>
                        </form>
                    </section>
                }
        }
            .render()
            .into_inner()
        });

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
                                <th>(t("Service"))</th>
                                // The copies. Headed by nothing, for the
                                // reason the delete column is: the cell
                                // says "3 replicas · 2 elsewhere", which
                                // names itself. It was headed "Address"
                                // and held one, until a service had more
                                // than one copy and the column started
                                // answering a different question.
                                <th class="address"></th>
                                <th class="state">(t("State"))</th>
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
                                        (cell.whereabouts.say())
                                    </td>
                                    <td class="state" data-state=(&service.id)>
                                        (state_badge(cell))
                                    </td>
                                    <td class="row">
                                        @if allowed.may_deploy() {
                                            (deploy_controls(
                                                &project.slug,
                                                &service.slug,
                                                cell,
                                                &format!("/projects/{}", project.slug),
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
    // Where to go afterwards. The form carries it because only the form
    // knows which page it is on — the same two controls live on the
    // list and on the service's own page.
    from: &'a str,
) -> impl Renderable + 'a {
    rsx! {
        <form method="post" data-action="deploy"
              class=(shown(cell.action == "deploy"))
              action=(format!("/projects/{project_slug}/services/{service_slug}/deploy"))>
            <input type="hidden" name="from" value=(from)>
            (action_button("Deploy", PLAY, cell.busy))
        </form>
        <form method="post" data-action="stop"
              class=(shown(cell.action == "stop"))
              action=(format!("/projects/{project_slug}/services/{service_slug}/stop"))>
            <input type="hidden" name="from" value=(from)>
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

/// Who is in this project, and how to change that.
///
/// A section of the settings page rather than a page of its own. It was
/// three unlabelled columns and a wide row of fields under a heading
/// that said "People" — a table with no headers reads as a list of
/// facts nobody explained, and the role was a word floating beside a
/// name with nothing saying what it was.
///
/// So: headers, the roles said in a sentence under the form, and the
/// add form as a labelled row rather than three controls in a line.
fn people_card<'a>(
    here: &'a str,
    members: &'a [access::Member],
    allowed: crate::accounts::roles::Access,
) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("People"))</p>
            @if members.is_empty() {
                <p class="tile-detail">(t("Nobody yet. Administrators of this node reach every project \
                     without being added to it."))</p>
            } @else {
                <table>
                    <thead>
                        <tr>
                            <th>(t("Person"))</th>
                            <th>(t("Can"))</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        @for member in members {
                            <tr>
                                <td>(&member.username)</td>
                                <td>
                                    <span class="badge">(member.role.label())</span>
                                    <span class="tile-detail">(role_means(member.role))</span>
                                </td>
                                <td class="row-actions">
                                    @if allowed.may_administer() {
                                        <form method="post"
                                              action=(format!(
                                                  "{here}/people/{}/remove", member.account_id
                                              ))>
                                            <button class="btn btn-ghost destructive btn-sm"
                                                    type="submit">(t("Remove"))</button>
                                        </form>
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            }

            @if allowed.may_administer() {
                <form method="post" action=(format!("{here}/people")) class="stack-sm">
                    <div class="add-person">
                        <div>
                            <label for="member">(t("Username"))</label>
                            // Named `member`, not `username`, and with
                            // no placeholder repeating the word.
                            //
                            // `autocomplete="off"` is a request, and
                            // Safari refuses it here: it decides a field
                            // is a login from the *name, id and
                            // placeholder*, and all three said
                            // `username`. So the keychain offered the
                            // operator their own account for a field
                            // asking who to add — reported twice, with
                            // the arrow pointing at it.
                            //
                            // The name is also the truer one. This is
                            // not your username; it is the person you
                            // are adding.
                            <input id="member" name="member" type="text" autocomplete="off"
                                   required>
                        </div>
                        <div>
                            <label for="role">(t("Can"))</label>
                            <select id="role" name="role">
                                @for role in ProjectRole::ALL {
                                    <option value=(role.as_str())>(role.label())</option>
                                }
                            </select>
                        </div>
                        <button type="submit">(t("Add"))</button>
                    </div>
                    <p class="field-hint">(t("Somebody who already has an account on this node — adding \
                         them here does not create one. A new person is invited from \
                         Settings, People."))</p>
                </form>
            }
        </section>
    }
}

/// What a role lets somebody do, in the words the page can afford.
fn role_means(role: ProjectRole) -> &'static str {
    match role {
        ProjectRole::Owner => "everything here, including who else is in it",
        ProjectRole::Deployer => "deploy and change services",
        ProjectRole::Viewer => "look, and change nothing",
    }
}

/// Everything on a project's pages that changes without a click.
///
/// Three maps rather than one, because they are keyed by three
/// different things and a page shows some or all of them: the overview
/// has services, the service page has services *and* its replicas and
/// its names.
///
/// It grew from the services map alone, and the gap was the point: a
/// replica placed on another node and a certificate being issued are
/// exactly the two things somebody presses a button and then waits for,
/// and both left the page frozen with a badge that had been true when
/// it rendered.
#[derive(serde::Serialize)]
pub(crate) struct Live {
    services: std::collections::BTreeMap<String, StateCell>,
    replicas: std::collections::BTreeMap<String, ReplicaCell>,
    names: std::collections::BTreeMap<String, NameCell>,
    edges: std::collections::BTreeMap<String, EdgeCell>,
}

impl Live {
    /// Every word in here, in the reader's language.
    ///
    /// The words are chosen where the state is decided — one place, so a
    /// first paint and an update cannot disagree — and that place has no
    /// business knowing who is reading. So they are English there and
    /// translated here, on the way out.
    fn translate(&mut self) {
        for cell in self.services.values_mut() {
            cell.word = super::language::word(&cell.word).to_string();
            // The one field composed rather than looked up. Said here
            // and rendered by `say` on the page, so a count sent two
            // seconds after a page load cannot be in the other
            // language.
            cell.address = cell.whereabouts.say();
        }
        for cell in self.replicas.values_mut() {
            cell.word = super::language::word(&cell.word).to_string();
            cell.detail = super::language::word(&cell.detail).to_string();
        }
        for cell in self.edges.values_mut() {
            cell.word = super::language::word(&cell.word).to_string();
        }
    }
}

/// One replica, as its row reads.
#[derive(serde::Serialize)]
pub(crate) struct ReplicaCell {
    pub(crate) badge: &'static str,
    pub(crate) dot: &'static str,
    pub(crate) word: String,
    /// What this copy is using, in millicores — a thousand is one core.
    ///
    /// Only for the copies here, and only from the second tick: a rate is
    /// the difference between two readings, which is why it lives on a
    /// stream rather than on a page render. Empty is "no answer yet", and
    /// the page leaves what it had rather than writing a dash over a
    /// figure.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) cpu: String,
    /// The line under it: an address while it is up, a reason while it
    /// is not, empty when there is nothing to add.
    pub(crate) detail: String,
}

/// One hostname, as far as its certificate has got.
#[derive(serde::Serialize)]
pub(crate) struct NameCell {
    /// Whether to show the "on the way" badge at all. The page renders
    /// it either way and this hides it — the same mechanism the deploy
    /// and stop controls use, so nothing has to be built client-side.
    waiting: bool,
}

/// One node asked to serve one name, as far as the asking has got.
#[derive(serde::Serialize)]
pub(crate) struct EdgeCell {
    badge: &'static str,
    dot: &'static str,
    /// Owned, because it is translated on the way out and the Spanish for
    /// a constant is a different constant. One allocation per edge per
    /// tick, against a second spelling of `language::word` for static
    /// text — which is the kind of pair that drifts.
    word: String,
}

/// How far each edge instruction has got.
///
/// Keyed `hostname|node`, which is what the row is: the same name can be
/// asked of several nodes and each answers on its own.
///
/// Read from the errand, because the errand is the only thing that
/// crosses. A tick used to be the end of what the page said — an errand
/// went out, a name was claimed and a certificate ordered, and none of
/// it came back — so the honest reading of a ticked box was "somebody
/// asked for this once".
async fn edge_cells(
    state: &super::ConsoleState,
    project: &crate::platform::projects::Project,
) -> std::collections::BTreeMap<String, EdgeCell> {
    let mut cells = std::collections::BTreeMap::new();
    let Ok(services) = crate::platform::services::all(&state.database, Some(&project.id)).await
    else {
        return cells;
    };
    let orders = crate::network::errand::all(&state.database)
        .await
        .unwrap_or_default();

    for service in services {
        let Ok(chosen) = crate::platform::edges::of_service(&state.database, &service.id).await
        else {
            continue;
        };
        for (hostname, node_id) in chosen {
            // The newest instruction to that node about that name.
            // Older ones are history and saying "done" from one of them
            // while a newer sits pending would be the page reporting the
            // wrong round trip.
            let latest = orders
                .iter()
                .filter(|order| order.node_id == node_id)
                .filter(|order| order.kind == crate::network::errand::Kind::Edge)
                .filter(|order| {
                    order
                        .payload
                        .get("hostname")
                        .and_then(|value| value.as_str())
                        == Some(hostname.as_str())
                })
                .max_by_key(|order| order.created_at);

            let cell = match latest {
                Some(order) if order.error.is_some() => EdgeCell {
                    badge: "badge badge-danger",
                    dot: "dot dot-danger",
                    word: "Refused".into(),
                },
                Some(order) if order.done() => EdgeCell {
                    badge: "badge badge-success",
                    dot: "dot dot-success",
                    word: "Serving".into(),
                },
                Some(_) => EdgeCell {
                    badge: "badge badge-info",
                    dot: "dot dot-info dot-pulse",
                    word: "Asked".into(),
                },
                // No errand at all: this node itself, which needs none.
                None => EdgeCell {
                    badge: "badge badge-success",
                    dot: "dot dot-success",
                    word: "Serving".into(),
                },
            };
            cells.insert(format!("{hostname}|{node_id}"), cell);
        }
    }
    cells
}

/// What every replica of every service in this project is doing.
async fn replica_cells(
    state: &super::ConsoleState,
    project: &crate::platform::projects::Project,
    // What each container here is using, from two readings of the stream.
    // `None` on the first tick, which is why the cell is empty rather than
    // a dash: the page keeps what it has instead of flashing.
    busy: Option<&crate::node::cpu::Busy>,
) -> std::collections::BTreeMap<String, ReplicaCell> {
    let mut cells = std::collections::BTreeMap::new();
    let Ok(services) = crate::platform::services::all(&state.database, Some(&project.id)).await
    else {
        return cells;
    };

    // Which nodes have an instruction waiting to be collected, once for the
    // project rather than once per replica.
    let queued = crate::network::errand::all(&state.database)
        .await
        .unwrap_or_default();

    for service in services {
        let Ok(replicas) =
            crate::platform::replicas::of_service(&state.database, &service.id).await
        else {
            continue;
        };
        let stopped = service.desired_state == crate::platform::services::DesiredState::Stopped;
        // Which of its copies the edge has taken out of the rotation.
        // Once per service rather than once per replica: it is one read
        // of the health map and one derivation of the addresses.
        let silent = state.deployer.not_answering(&service).await;
        for replica in replicas {
            let waiting_for_it = replica.node_id.as_ref().is_some_and(|node| {
                queued.iter().any(|record| {
                    record.done_at.is_none()
                        && &record.node_id == node
                        && record.payload.get("service").and_then(|name| name.as_str())
                            == Some(service.name.as_str())
                })
            });
            let mut cell = replica_cell(
                &replica,
                stopped,
                waiting_for_it,
                silent.contains(&replica.id),
            );
            // Only the copies here. One elsewhere is measured by the node
            // running it and arrives on its report, which the server has
            // already rendered — writing over it from a stream that has no
            // reading for it would blank a figure that is true.
            if replica.is_here() && !replica.evicted() {
                let id = replica.container_id(&project.slug, &service.slug);
                if let Some(share) = busy.and_then(|busy| busy.containers.get(&id)) {
                    // A thousand is one core. Absolute, so this column can
                    // be read against the same column on another node.
                    cell.cpu = format!("{share}m");
                }
            }
            cells.insert(replica.id.clone(), cell);
        }
    }
    cells
}

/// What one replica's row says — the **only** decision, read by the first
/// paint and by every update after it.
///
/// The comment here used to say the words were "kept beside"
/// `placement_state` deliberately, and name the failure that would cause:
/// a page saying one thing on load and another two seconds later. It then
/// happened exactly as described. A stopped service's remote copy was
/// given its own word in `placement_state` and not here, so the first
/// paint said "not running" and the stream put "waiting for that node"
/// back over it — reported by Jorge with the two rows beside each other,
/// the local copies right and the remote one wrong.
///
/// So `placement_state` renders *this*, and there is nothing left to keep
/// in step.
pub(crate) fn replica_cell(
    replica: &crate::platform::replicas::Replica,
    // Whether the service is meant to be running at all. Nobody waits for
    // news about a copy that is not, here or anywhere else.
    stopped: bool,
    // Whether an instruction about it is still waiting to be collected,
    // which is a different thing from silence.
    queued: bool,
    // Whether the edge has taken it out of the rotation: its container
    // is up and nothing answers on the port. A distinct state from every
    // other one here, and the one somebody is looking for when a service
    // is "up" and losing requests.
    not_answering: bool,
) -> ReplicaCell {
    if replica.evicted() {
        return ReplicaCell {
            badge: "badge badge-warning",
            dot: "",
            word: "Evicted there".into(),
            cpu: String::new(),
            detail: String::new(),
        };
    }
    if let Some(failure) = &replica.last_error {
        return ReplicaCell {
            badge: "badge badge-danger",
            dot: "dot dot-danger",
            word: "Failed".into(),
            cpu: String::new(),
            detail: failure.clone(),
        };
    }
    // Running, and no longer replicating. **Above the rotation check and
    // below the failures**, which is where it belongs: nothing failed —
    // the container is up and answering — and what is wrong is that its
    // data stopped moving. A copy somebody reads from that is silently
    // hours out of date is worse than one that is down, because down is
    // visible.
    //
    // Only ever `Some(false)`: `None` is "not asked yet", and rendering
    // this for a copy nobody has asked about would be the page inventing
    // an outage.
    if replica.following == Some(false) && !replica.evicted() {
        return ReplicaCell {
            badge: "badge badge-danger",
            dot: "dot dot-danger",
            word: "Not following".into(),
            cpu: String::new(),
            detail: match replica.wal_held {
                Some(held) if held > 0 => format!(
                    "{} of write-ahead log held for it",
                    crate::node::memory::human(held)
                ),
                _ => String::new(),
            },
        };
    }
    if let Some(address) = &replica.address {
        // Running and out of the rotation. Not `Failed`, which is what
        // the *node* said about starting it, and not `Running`, which
        // would be the page agreeing with a container that is lying: the
        // process is there and nothing answers on the port it was given.
        // Warning rather than danger, because the edge has already moved
        // the traffic — this is somebody to tell, not an outage.
        if not_answering {
            return ReplicaCell {
                badge: "badge badge-warning",
                dot: "dot dot-warning",
                word: "Not answering".into(),
                cpu: String::new(),
                detail: address.clone(),
            };
        }
        return ReplicaCell {
            badge: "badge badge-success",
            dot: "dot dot-success",
            word: "Running".into(),
            cpu: String::new(),
            detail: address.clone(),
        };
    }
    if replica.is_here() || stopped {
        return ReplicaCell {
            badge: "badge",
            dot: "",
            word: "Not running".into(),
            cpu: String::new(),
            detail: String::new(),
        };
    }
    match queued {
        // Written and not yet collected, which is most of what makes a
        // wait feel long.
        true => ReplicaCell {
            badge: "badge badge-info",
            dot: "dot dot-info dot-pulse",
            word: "Queued for that node".into(),
            cpu: String::new(),
            detail: String::new(),
        },
        // Told, and nothing has come back. The node reports when it
        // collects, so this is the honest word until it does.
        false => ReplicaCell {
            badge: "badge badge-info",
            dot: "dot dot-info dot-pulse",
            word: "Waiting for that node".into(),
            cpu: String::new(),
            detail: String::new(),
        },
    }
}

/// Which of this project's hostnames are still waiting for a
/// certificate.
///
/// The badge under an address used to be true only at the moment the
/// page rendered, and an ACME order takes minutes — so the one thing
/// somebody stares at after adding a hostname was the one thing that
/// never changed on its own.
async fn name_cells(
    state: &super::ConsoleState,
    project: &crate::platform::projects::Project,
) -> std::collections::BTreeMap<String, NameCell> {
    let mut cells = std::collections::BTreeMap::new();
    let Ok(services) = crate::platform::services::all(&state.database, Some(&project.id)).await
    else {
        return cells;
    };

    for service in services {
        let Ok(ports) = crate::platform::ports::of_service(&state.database, &service.id).await
        else {
            continue;
        };
        for hostname in ports.into_iter().filter_map(|port| port.hostname) {
            let secured = crate::edge::certs::load(&state.database, &hostname)
                .await
                .ok()
                .flatten()
                .is_some();
            cells.insert(hostname, NameCell { waiting: !secured });
        }
    }
    cells
}

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
    /// Where it runs, in the reader's language.
    ///
    /// Written by [`LiveState::translate`], which is the only thing that
    /// reads it: the page composes the same sentence from
    /// [`StateCell::whereabouts`] inside its own render scope. Two
    /// callers of one function rather than one value passed through two
    /// places that would each have to remember to translate it.
    address: String,
    /// The parts of that sentence, before it is one.
    ///
    /// A count and a language are decided in different places — the
    /// count in a handler, which awaits, and the language around a
    /// render, which cannot. So the handler carries the count and the
    /// render says it.
    #[serde(skip)]
    pub(crate) whereabouts: Whereabouts,
}

/// How many copies of a service there are, and how many of them are on
/// other machines.
///
/// A count rather than an address, everywhere. The single-copy case used
/// to name the container's address, which put a `10.42.x.x` under a
/// column headed "Address" on one row and a count on the next — and that
/// address is reachable from inside the project only, where the
/// service's own name resolves and is the thing to use. Where each copy
/// is and what it is doing belongs to the replicas table, one row each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Whereabouts {
    pub(crate) total: usize,
    pub(crate) away: usize,
}

impl Whereabouts {
    /// The sentence, in the language of the render in progress.
    ///
    /// The number is a number in both languages and the nouns are not,
    /// so the nouns go through `t` and the sentence is composed around
    /// them. It was composed whole in the handler and reached neither
    /// door — `t` takes a literal and this is not one, and the stream's
    /// `translate` looks a whole value up in a table, which no sentence
    /// with a count in it can ever be found in. So a Spanish console
    /// read "3 replicas · 2 elsewhere".
    pub(crate) fn say(&self) -> String {
        match (self.total, self.away) {
            (0, _) => t("nowhere").to_string(),
            // A copy that is not here is worth saying even when it is
            // the only one: "running" on a page served by this node
            // would otherwise mean a container it has never seen.
            (1, 1) => format!("1 {}", t("elsewhere")),
            (1, _) => format!("1 {}", t("replica")),
            (total, 0) => format!("{total} {}", t("replicas")),
            (total, away) => format!("{total} {} · {away} {}", t("replicas"), t("elsewhere")),
        }
    }
}

pub(crate) fn where_it_runs(placements: &[crate::platform::replicas::Replica]) -> Whereabouts {
    let live: Vec<&crate::platform::replicas::Replica> =
        placements.iter().filter(|r| !r.evicted()).collect();

    Whereabouts {
        total: live.len(),
        away: live.iter().filter(|r| !r.is_here()).count(),
    }
}

/// What the service's own node can see, when nothing of it runs there.
///
/// A service placed entirely on other machines used to read `Absent`
/// here, which the badge says as **"Not deployed"** — and it is deployed,
/// it is running, just not on the machine being asked. Reported by Jorge
/// with the page beside it: a copy `Running` on another node, and a badge
/// over it saying the service had never been deployed.
///
/// Two answers rather than one, because the page must not invent an
/// outcome nobody reported: a copy elsewhere is only known to be up once
/// that node has said so, which it does when it collects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Elsewhere {
    /// Some copy on another node has reported an address.
    Running,
    /// Placed, and nothing has come back about it yet.
    Silent,
}

/// What the copies on other nodes amount to, when none of them is here.
///
/// `None` the moment a copy runs on this node: `observed` is the answer
/// then, and this is not. One definition because three places ask —
/// the project's list, its stream, and the service's own page — and a
/// badge that disagreed with itself between a first paint and the swap
/// that followed is the failure `StateCell` exists to prevent.
pub(crate) fn elsewhere_of(placements: &[crate::platform::replicas::Replica]) -> Option<Elsewhere> {
    let live = placements.iter().filter(|replica| !replica.evicted());
    if live.clone().any(|replica| replica.is_here()) {
        return None;
    }
    let mut away = live.filter(|replica| !replica.is_here()).peekable();
    away.peek()?;
    match away.any(|replica| replica.address.is_some()) {
        true => Some(Elsewhere::Running),
        false => Some(Elsewhere::Silent),
    }
}

pub(crate) fn state_cell(
    observed: &Observed,
    deploying: bool,
    whereabouts: Whereabouts,
    // `None` when copies run on this node, which is the ordinary case and
    // the one `observed` already answers for.
    elsewhere: Option<Elsewhere>,
) -> StateCell {
    // Empty until something says it: the sentence needs a language, and
    // this runs in a handler, which has none around it.
    let address = String::new();
    if deploying {
        return StateCell {
            word: "Deploying".into(),
            badge: "badge badge-info",
            dot: "dot dot-info dot-pulse",
            action: "stop",
            busy: true,
            address,
            whereabouts,
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
            whereabouts,
        },
        Observed::Stopped { exit_code } => StateCell {
            word: format!("Exited {exit_code}"),
            badge: "badge badge-danger",
            dot: "dot dot-danger",
            action: "deploy",
            busy: false,
            address: address.clone(),
            whereabouts: whereabouts.clone(),
        },
        // Nothing here. Which is not the same as nothing anywhere, and
        // the difference is the whole of what this argument is for — the
        // action included: stopping it is what its owner can do about a
        // service running on somebody else's machine, and the instruction
        // travels now.
        Observed::Absent => match elsewhere {
            Some(Elsewhere::Running) => StateCell {
                word: "Running elsewhere".into(),
                badge: "badge badge-success",
                dot: "dot dot-success",
                action: "stop",
                busy: false,
                address: address.clone(),
                whereabouts: whereabouts.clone(),
            },
            Some(Elsewhere::Silent) => StateCell {
                word: "Waiting for that node".into(),
                badge: "badge badge-info",
                dot: "dot dot-info dot-pulse",
                action: "stop",
                busy: false,
                address: address.clone(),
                whereabouts: whereabouts.clone(),
            },
            None => StateCell {
                word: "Not deployed".into(),
                badge: "badge badge-warning",
                dot: "dot dot-warning",
                action: "deploy",
                busy: false,
                address: address.clone(),
                whereabouts: whereabouts.clone(),
            },
        },
        Observed::Unknown(_) => StateCell {
            word: "Unknown".into(),
            badge: "badge badge-info",
            dot: "dot dot-info",
            action: "deploy",
            busy: false,
            address: address.clone(),
            whereabouts: whereabouts.clone(),
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
            <span class=(cell.dot)></span>(super::language::word(&cell.word))
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
            // The previous CPU reading. A percentage is the difference
            // between two, and a stream is the one place that has both —
            // which is why this figure is on the table and not on the page
            // render beside memory and disk.
            let mut before: Option<crate::node::cpu::Sample> = None;
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
                        let placements = crate::platform::replicas::of_service(
                            &state.database,
                            &service.id,
                        )
                        .await
                        .unwrap_or_default();
                        cells.insert(
                            service.id.clone(),
                            state_cell(
                                &observed,
                                busy,
                                where_it_runs(&placements),
                                elsewhere_of(&placements),
                            ),
                        );
                    }
                }
                let now = crate::node::cpu::sample(&state.deployer.cgroups().await);
                let busy = before
                    .as_ref()
                    .and_then(|before| crate::node::cpu::between(before, &now));
                before = Some(now);

                let mut live = Live {
                    services: cells,
                    replicas: replica_cells(&state, &project, busy.as_ref()).await,
                    names: name_cells(&state, &project).await,
                    edges: edge_cells(&state, &project).await,
                };
                // Into the reader's language, after the reads and before
                // the write — the same shape a view uses, and for the same
                // reason: the scope is a thread-local around a stretch with
                // no `await` in it.
                //
                // Without this the badge a page rendered in Spanish turned
                // back to English two seconds later, which is the same
                // "decided in two places" failure as the words themselves,
                // one layer down.
                super::language::scoped(account.language, || {
                    live.translate();
                    String::new()
                });
                let payload = serde_json::to_string(&live).unwrap_or_else(|_| "{}".into());
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
        let username = field(&form, "member");
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

    /// The column counts copies and never names an address.
    ///
    /// It named one while there was exactly one copy, which made the
    /// column answer two different questions a row apart — an address on
    /// this row, a count on the next, under a heading that could only
    /// suit one of them. And the address it named is reachable from
    /// inside the project only, where the service's own name resolves
    /// and is the thing to use. Which copy is where is the replicas
    /// table, a row each.
    #[test]
    fn the_column_counts_copies_rather_than_naming_one() {
        let replica =
            |slot, here: bool, address: Option<&str>| crate::platform::replicas::Replica {
                id: format!("rp-{slot}"),
                service_id: "svc-1".into(),
                node_id: (!here).then(|| "nd-elsewhere".to_string()),
                slot,
                address: address.map(str::to_string),
                overlay_port: None,
                last_error: None,
                evicted_at: None,
                reserved_host: None,
                memory_bytes: None,
                disk_bytes: None,
                cpu_millicores: None,
                following: None,
                wal_held: None,
            };

        // The ordinary case: one copy, here, with an address the page
        // no longer shows.
        assert_eq!(
            where_it_runs(&[replica(1, true, Some("10.42.1.5"))]).say(),
            "1 replica"
        );
        // One copy and it is not on this machine, which is worth saying
        // even alone: "running" here would otherwise mean a container
        // this node has never seen.
        assert_eq!(
            where_it_runs(&[replica(1, false, None)]).say(),
            "1 elsewhere"
        );
        // Placed nowhere at all — not "0 replicas", which reads as a
        // measurement of something.
        assert_eq!(where_it_runs(&[]).say(), "nowhere");

        assert_eq!(
            where_it_runs(&[replica(1, true, Some("10.42.1.5")), replica(2, true, None)]).say(),
            "2 replicas"
        );
        assert_eq!(
            where_it_runs(&[
                replica(1, true, Some("10.42.1.5")),
                replica(2, false, None),
                replica(3, false, None),
            ])
            .say(),
            "3 replicas · 2 elsewhere"
        );
    }

    /// The count is a sentence, and a sentence on this console is read
    /// in the account's language. It was composed in the handler out of
    /// English words and reached neither door — not `t`, which takes a
    /// literal, and not the stream's `translate`, which looks a whole
    /// value up in a table. So a Spanish page said "3 replicas · 2
    /// elsewhere", in the one column that changes as copies move.
    #[test]
    fn the_count_of_copies_is_read_in_the_accounts_language() {
        let three = Whereabouts { total: 3, away: 2 };
        let spanish =
            crate::console::language::scoped(crate::console::language::Language::Es, || {
                three.say()
            });
        assert_eq!(spanish, "3 réplicas · 2 en otro nodo");

        assert_eq!(
            crate::console::language::scoped(crate::console::language::Language::Es, || {
                Whereabouts { total: 1, away: 1 }.say()
            }),
            "1 en otro nodo"
        );
        // Singular, and the plural is not "1 replicas".
        assert_eq!(
            crate::console::language::scoped(crate::console::language::Language::Es, || {
                Whereabouts { total: 1, away: 0 }.say()
            }),
            "1 réplica"
        );
    }

    /// The landing page says what needs somebody, and says nothing when
    /// nothing does.
    ///
    /// The second half is the property that makes the first worth
    /// anything: a panel that is always on the page is wallpaper by the
    /// second week, and the day it has something to say it says it in
    /// the same place it said "all well".
    #[tokio::test]
    async fn the_landing_page_says_what_needs_somebody() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let quiet = console
            .harness
            .get("/")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            !quiet.contains("Needs you"),
            "nothing is wrong, so there is no panel: {quiet}"
        );

        // A copy that will not start — the most direct of the concerns,
        // and the one an operator most needs off the page they land on.
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "demo")])
            .send()
            .await;
        console
            .harness
            .post("/projects/demo/services")
            .header("cookie", &cookie)
            .form(&[("name", "api"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;
        let service = crate::platform::services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("made");
        let replica = crate::platform::replicas::ensure_here(&console.database, &service.id, 1)
            .await
            .expect("a copy")
            .pop()
            .expect("one");
        crate::platform::replicas::set_last_error(
            &console.database,
            &replica.id,
            Some("it started and exited 1"),
        )
        .await
        .expect("failed");

        let loud = console
            .harness
            .get("/")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(loud.contains("Needs you"), "{loud}");
        assert!(loud.contains("api #1"), "which one: {loud}");
        // Not *what* the reason says: that text is written by whatever
        // failed — the socket here, a container's last words on a node —
        // and the deployment this replica just started is free to
        // replace it while the page renders. That it is one line is
        // `a_reason_is_one_line_in_a_list`, which asserts it without
        // racing anything.
        assert!(
            loud.contains("/projects/demo/services/api"),
            "and somewhere to go: {loud}"
        );
    }

    /// A badge is prose, and the page around it is in the account's
    /// language — so a Spanish console with an English badge on it is the
    /// page contradicting itself, which is what Jorge's screenshot showed.
    ///
    /// The source scan in `es.rs` cannot find these: they are picked here
    /// as values and carried to the render on a `StateCell`, not asked for
    /// with `t`. So this is the guard, and it has to name every word the
    /// function can produce — bar `Exited n`, which is containerd's word
    /// and a number.
    #[test]
    fn a_state_word_is_a_word_somebody_reads() {
        // This test is about the word; where it runs is not part of it.
        let nowhere = || Whereabouts { total: 0, away: 0 };
        let placed = [
            state_cell(&Observed::Absent, true, nowhere(), None),
            state_cell(&Observed::Absent, false, nowhere(), None),
            state_cell(
                &Observed::Absent,
                false,
                nowhere(),
                Some(Elsewhere::Running),
            ),
            state_cell(&Observed::Absent, false, nowhere(), Some(Elsewhere::Silent)),
            state_cell(
                &Observed::Running {
                    pid: 1,
                    address: None,
                },
                false,
                nowhere(),
                None,
            ),
            state_cell(&Observed::Unknown("busy".into()), false, nowhere(), None),
        ];

        for cell in &placed {
            assert!(
                crate::console::es::lookup(&cell.word).is_some(),
                "no Spanish for the state word {:?}",
                cell.word
            );
        }

        // And the words of a *replica*, which reach a render the same
        // way and were guarded by nothing. They happened to be
        // translated; the first one added after this test was written —
        // "Not answering" — was not, and the whole suite stayed green.
        // A guard that covers one of two functions producing the same
        // kind of value is a guard that will be right until somebody
        // touches the other one.
        let replica = |address: Option<&str>| crate::platform::replicas::Replica {
            id: "rp-1".into(),
            service_id: "svc-1".into(),
            node_id: Some("nd-far".into()),
            slot: 2,
            address: address.map(str::to_string),
            overlay_port: None,
            last_error: None,
            evicted_at: None,
            reserved_host: None,
            memory_bytes: None,
            disk_bytes: None,
            cpu_millicores: None,
            following: None,
            wal_held: None,
        };
        let failed = crate::platform::replicas::Replica {
            last_error: Some("it exited 1".into()),
            ..replica(None)
        };
        let evicted = crate::platform::replicas::Replica {
            evicted_at: Some(1),
            ..replica(None)
        };

        let not_following = crate::platform::replicas::Replica {
            following: Some(false),
            ..replica(Some("10.42.1.5"))
        };
        let copies = [
            replica_cell(&evicted, false, false, false),
            replica_cell(&failed, false, false, false),
            replica_cell(&replica(Some("10.42.1.5")), false, false, false),
            replica_cell(&replica(Some("10.42.1.5")), false, false, true),
            replica_cell(&not_following, false, false, false),
            replica_cell(&replica(None), false, false, false),
            replica_cell(&replica(None), true, false, false),
            replica_cell(&replica(None), false, true, false),
        ];
        for cell in &copies {
            assert!(
                crate::console::es::lookup(&cell.word).is_some(),
                "no Spanish for the replica word {:?}",
                cell.word
            );
        }
    }

    /// During a deployment containerd's answer is a half-truth — the
    /// old container is gone and the new one is not up — so "absent"
    /// there reads as a fault. The job is what knows.
    #[test]
    fn a_deployment_in_flight_outranks_what_containerd_says() {
        let nowhere = || Whereabouts { total: 0, away: 0 };
        let busy = state_cell(&Observed::Absent, true, nowhere(), None);
        assert_eq!(busy.word, "Deploying");
        // Shown, not hidden: a control that vanishes takes the column's
        // width with it. Disabled says the same thing and keeps the row
        // still — and it names where the deployment is heading.
        assert_eq!(busy.action, "stop");
        assert!(busy.busy, "and it cannot be pressed yet");

        let idle = state_cell(&Observed::Absent, false, nowhere(), None);
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
            Whereabouts { total: 2, away: 1 },
            None,
        );
        assert_eq!(running.action, "stop");
        // Where it runs rides with the state: a deployment ends by
        // placing copies, and a row showing the new state beside the
        // old count is half-updated.
        assert_eq!(running.whereabouts.say(), "2 replicas · 1 elsewhere");
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

        // Everything about the project rather than the work on it, on
        // one page. People used to be a page of its own holding a
        // single table; it is a section here now, and the old address
        // redirects rather than 404s — it was a nav item for months, so
        // it is in somebody's history.
        let settings = get("/projects/shipping/settings").await;
        assert!(settings.contains("Push tokens"), "{settings}");
        assert!(settings.contains("Danger zone"), "{settings}");
        // Not empty: creating a project grants the creator Owner, so
        // there is always at least one person to list.
        assert!(settings.contains("Owner"), "{settings}");
        assert!(settings.contains("Username"), "and a way to add one");
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
                vec![("member", "nobody"), ("role", "viewer")],
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
