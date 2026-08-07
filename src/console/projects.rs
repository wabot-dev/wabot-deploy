//! Projects: the list, the create form, and one project's page.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
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
    /// A push token, shown once by the page that just made it.
    pub token: Option<String>,
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
        let facts = super::certificate_facts(&self.state).await;

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
                    <section class="empty">
                        <p>("No projects yet.")</p>
                        <a class="btn" href="/projects/new">("Create project")</a>
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

                (super::node_card(&facts))
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
        let mut rows = Vec::with_capacity(services.len());
        for service in services {
            let observed = self.state.deployer.observe(&project, &service).await;
            rows.push((service, observed));
        }

        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let members = access::members(&self.state.database, &project.id).await?;
        let tokens = if allowed.may_deploy() {
            crate::platform::tokens::of_project(&self.state.database, &project.id).await?
        } else {
            Vec::new()
        };
        let registry_host = self
            .state
            .config
            .node
            .domain
            .clone()
            .unwrap_or_else(|| "this node".into());
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
                    <table>
                        <thead>
                            <tr>
                                <th>("Service")</th>
                                <th>("Image")</th>
                                <th>("Address")</th>
                                <th>("State")</th>
                                // The delete column. Headed by nothing,
                                // because "Delete" over a column of
                                // Delete buttons reads as a heading for
                                // a thing rather than a label for one.
                                <th></th>
                            </tr>
                        </thead>
                        <tbody>
                            @for (service, observed) in &rows {
                                <tr>
                                    <td>
                                        <a href=(format!(
                                            "/projects/{}/services/{}",
                                            project.slug, service.slug
                                        ))>(&service.name)</a>
                                    </td>
                                    <td class="mono">(&service.image)</td>
                                    <td class="mono">(
                                        service.address.clone().unwrap_or_else(|| "—".into())
                                    )</td>
                                    <td>(state_badge(observed))</td>
                                    <td class="row">
                                        @if !allowed.may_deploy() {
                                            // A viewer sees the state and
                                            // nothing to press. The check
                                            // that matters is on the POST;
                                            // this is so the page does not
                                            // offer what it will refuse.
                                        } @else if matches!(observed, Observed::Running { .. }) {
                                            <form method="post"
                                                  action=(format!(
                                                      "/projects/{}/services/{}/stop",
                                                      project.slug, service.slug
                                                  ))>
                                                <button class="btn btn-secondary btn-sm" type="submit">
                                                    ("Stop")
                                                </button>
                                            </form>
                                        } @else {
                                            <form method="post"
                                                  action=(format!(
                                                      "/projects/{}/services/{}/deploy",
                                                      project.slug, service.slug
                                                  ))>
                                                <button class="btn btn-secondary btn-sm"
                                                        type="submit">
                                                    ("Deploy")
                                                </button>
                                            </form>
                                        }
                                        @if allowed.may_deploy() {
                                            <form method="post"
                                                  action=(format!(
                                                      "/projects/{}/services/{}/delete",
                                                      project.slug, service.slug
                                                  ))>
                                                <button class="btn btn-ghost destructive btn-sm"
                                                        type="submit">
                                                    ("Delete")
                                                </button>
                                            </form>
                                        }
                                    </td>
                                </tr>
                                @if let Some(failure) = &service.last_error {
                                    <tr>
                                        <td colspan="5" class="failure">(failure)</td>
                                    </tr>
                                }
                            }
                        </tbody>
                    </table>
                }

                @if allowed.may_deploy() {
                <section class="card stack">
                    <div class="split">
                        <p class="card-label">("Push tokens")</p>
                        <span class="who">(&registry_host)("/")(&project.slug)("/…")</span>
                    </div>
                    @if let Some(secret) = &query.token {
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
                                                      "/projects/{}/tokens/{}/revoke",
                                                      project.slug, token.id
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
                    <form method="post" action=(format!("/projects/{}/tokens", project.slug))
                          class="row">
                        <input name="name" type="text" placeholder="what it is for" required>
                        <button class="btn btn-secondary" type="submit">("Create token")</button>
                    </form>
                </section>
                }

                <section class="card stack">
                    <div class="split">
                        <p class="card-label">("People")</p>
                        <span class="who">("You are: ")(allowed.label())</span>
                    </div>
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
                                                      "/projects/{}/people/{}/remove",
                                                      project.slug, member.account_id
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
                        <form method="post"
                              action=(format!("/projects/{}/people", project.slug))
                              class="row">
                            <input name="username" type="text" placeholder="username" required>
                            <select name="role">
                                @for role in ProjectRole::ALL {
                                    <option value=(role.as_str())>(role.label())</option>
                                }
                            </select>
                            <button class="btn btn-secondary" type="submit">("Add")</button>
                        </form>
                        <p class="field-hint">(
                            "Somebody who already has an account on this node. To bring \
                             in a new person, invite them from the people page."
                        )</p>
                    }
                </section>

                @if allowed.may_administer() {
                <section class="card stack">
                    <p class="card-label">("Danger zone")</p>
                    <p class="tile-detail">(
                        "Deleting a project deletes every service under it. \
                         Nothing is stopped first — do that yourself."
                    )</p>
                    <form method="post"
                          action=(format!("/projects/{}/delete", project.slug))>
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
pub(crate) fn state_badge(observed: &Observed) -> impl Renderable + '_ {
    rsx! {
        @match observed {
            Observed::Running { pid, .. } => {
                <span class="badge badge-success" title=(format!("pid {pid}"))>
                    <span class="dot dot-success"></span>("Running")
                </span>
            }
            Observed::Stopped { exit_code } => {
                <span class="badge badge-danger">
                    <span class="dot dot-danger"></span>("Exited ")(exit_code)
                </span>
            }
            Observed::Absent => {
                <span class="badge badge-warning">
                    <span class="dot dot-warning"></span>("Not deployed")
                </span>
            }
            Observed::Unknown(_) => {
                <span class="badge badge-info">
                    <span class="dot dot-info"></span>("Unknown")
                </span>
            }
        }
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
    #[post("/projects/:project/people")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add_member(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+project"));
        };
        let here = format!("/projects/{}", project.slug);
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
        let here = format!("/projects/{}", project.slug);
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
        let here = format!("/projects/{}", project.slug);
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
            // Through the query string, because this is the only time
            // the secret exists in clear. Stored hashed, so no page can
            // ever show it again.
            Ok((_, secret)) => Ok(see_other(&format!(
                "{here}?{}",
                form_urlencoded::Serializer::new(String::new())
                    .append_pair("token", &secret)
                    .finish()
            ))),
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
        let here = format!("/projects/{}", project.slug);
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
            .get("/projects/mine")
            .header("cookie", &member)
            .send()
            .await
            .body;
        assert!(body.contains("Danger zone"), "an owner sees it: {body}");
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
        assert!(!body.contains("Danger zone"));

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
