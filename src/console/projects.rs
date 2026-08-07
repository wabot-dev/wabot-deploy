//! Projects: the list, the create form, and one project's page.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use crate::deploy::Observed;
use crate::platform::{projects, services};

use super::auth::{
    back_with_error, field, read_form, see_other, signed_in, PageQuery, SessionMiddleware,
};
use super::shell::{Area, Frame};
use super::{layout, ConsoleState};

#[derive(Debug, Deserialize, Validate)]
pub struct ProjectQuery {
    pub project: String,
    pub error: Option<String>,
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

        let projects = projects::all(&self.state.database).await?;
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

        let projects = projects::all(&self.state.database).await?;

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

        let Some(project) = projects::find(&self.state.database, &query.project).await? else {
            // Back to the list saying so, rather than a 404 page. The
            // only way here is a stale link or a deleted project, and
            // in both cases the list is what they wanted.
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

        let all_projects = projects::all(&self.state.database).await?;
        let path = format!("/projects/{}", project.slug);

        layout::head(&project.name);
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            path,
        );
        let body = rsx! {
                (layout::style_tag())
                <div class="split">
                    <div class="stack-sm">
                        <h1>(&project.name)</h1>
                        <p class="slug-preview">(&project.slug)</p>
                    </div>
                    <a class="btn" href=(&new_service)>("Create service")</a>
                </div>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }

                @if rows.is_empty() {
                    <section class="empty">
                        <p>("No services yet.")</p>
                        <a class="btn" href=(&new_service)>("Create service")</a>
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
                                        @if matches!(observed, Observed::Running { .. }) {
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
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
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

        match projects::create(&self.state.database, name).await {
            Ok(project) => Ok(see_other(&format!("/projects/{}", project.slug))),
            Err(error) => Ok(back_with_error("/projects/new", &error.to_string())),
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

        match projects::find(&self.state.database, slug).await? {
            Some(project) => Ok(see_other(&format!("/projects/{}", project.slug))),
            None => Ok(see_other("/?error=no+such+project")),
        }
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
        let segments = super::auth::segments(&path);
        let Some(slug) = segments.get(1) else {
            return Ok(see_other("/?error=no+such+project"));
        };
        let Some(project) = projects::find(&self.state.database, slug).await? else {
            // Already gone. The operator wanted it gone, and it is.
            return Ok(see_other("/"));
        };

        projects::delete(&self.state.database, &project.id).await?;
        Ok(see_other("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

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
