//! What a project's forms post to.

use crate::console::live::{edge_cells, name_cells, replica_cells, Live};
use std::sync::Arc;

use crate::views::components::service_state::{elsewhere_of, state_cell, where_it_runs};
use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::accounts::roles::ProjectRole;
use crate::policies::access;
use crate::repositories::services;

use crate::console::auth::{
    back_with_error, field, read_form, see_other, signed_in, SessionMiddleware,
};
use crate::console::ConsoleState;

impl ProjectRestController {
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
            crate::entities::project::Project,
            crate::accounts::roles::Access,
            Vec<&'a str>,
        )>,
    > {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(None);
        };
        let segments = crate::console::auth::segments(path);
        let Some(slug) = segments.get(1) else {
            return Ok(None);
        };
        Ok(access::find_project(&self.state.database, &account, slug)
            .await?
            .map(|(project, allowed)| (project, allowed, segments)))
    }
}

#[injectable]
pub struct ProjectRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl ProjectRestController {
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

        let made = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_create_project::run(
                    &self.state.database,
                    &account,
                    name,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_create_project::run(
                    &self.state.database,
                    &account,
                    name,
                )
                .await
            }
        };
        match made {
            Ok(project) => Ok(see_other(&format!("/projects/{}", project.slug))),
            Err(reason) => Ok(back_with_error("/projects/new", &reason)),
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
        let Some(slug) = crate::console::auth::segments(&path)
            .get(1)
            .map(|s| s.to_string())
        else {
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
                        let placements = crate::repositories::replicas::of_service(
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
                crate::views::texts::language::scoped(account.language, || {
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
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let username = field(&form, "member");
        let role = ProjectRole::parse(field(&form, "role"));

        let added = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_add_project_member::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    username,
                    role,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_add_project_member::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    username,
                    role,
                )
                .await
            }
        };
        match added {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
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
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let Some(person) = segments.get(3) else {
            return Ok(see_other(&here));
        };

        let removed = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_remove_project_member::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    person,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_remove_project_member::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    person,
                )
                .await
            }
        };
        match removed {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
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
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let minted = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_create_project_token::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    field(&form, "name"),
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_create_project_token::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    field(&form, "name"),
                )
                .await
            }
        };
        match minted {
            // The nonce travels, not the secret. This is the only
            // moment the token exists in clear — it is stored hashed,
            // so no page can ever show it again — and a query string is
            // read by the address bar, the history and every refresh.
            // See `ConsoleState::reveals`.
            Ok(secret) => {
                // **The command, not the bare secret.** What the store
                // holds is what goes on the clipboard — the reveal page
                // chooses the words around a value, never the value —
                // and a push token on its own is something somebody has
                // to remember the wrapper for.
                let host = crate::node::settings::domain(&self.state.database, &self.state.config)
                    .await
                    .unwrap_or_else(|| "this node".into());
                let nonce = self.state.reveals.stash(format!(
                    "docker login {host} -u {} -p {secret}",
                    field(&form, "name")
                ));
                Ok(see_other(&format!(
                    "/projects/{}/shown?{}",
                    project.slug,
                    form_urlencoded::Serializer::new(String::new())
                        .append_pair("nonce", &nonce)
                        .finish()
                )))
            }
            Err(reason) => Ok(back_with_error(&here, &reason)),
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
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let Some(id) = segments.get(3) else {
            return Ok(see_other(&here));
        };

        let revoked = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_revoke_project_token::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    id,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_revoke_project_token::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    id,
                )
                .await
            }
        };
        match revoked {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    /// Delete a project and, by cascade, its services.
    ///
    /// A POST, so nothing a browser prefetches can trigger it, and the
    /// project's name has to be typed.
    ///
    /// **There used to be no confirmation at all**, and the reasoning
    /// written here for that was: a dialog needs JavaScript, this console
    /// works without it, and the "Danger zone" heading is the warning.
    /// Half right, and the wrong conclusion — a *dialog* needs scripting,
    /// a text field does not, and a heading is not a warning. This
    /// deletes every service under the project. Reported by Jorge, who
    /// pressed the equivalent button on a service.
    ///
    /// Checked here rather than trusted from the form: `required` is a
    /// courtesy the browser extends, and a request can be constructed by
    /// hand.
    #[post("/projects/:project/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some((project, allowed, _)) = self.locate(&path).await? else {
            // Already gone, or never theirs. Both answer the same.
            return Ok(see_other("/"));
        };
        let settings = format!("/projects/{}/settings", project.slug);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let deleted = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_delete_project::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &allowed,
                    field(&form, "confirm"),
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_delete_project::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &allowed,
                    field(&form, "confirm"),
                )
                .await
            }
        };
        match deleted {
            Ok(()) => Ok(see_other("/")),
            Err(reason) => Ok(back_with_error(&settings, &reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;
    use crate::repositories::projects;
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
        // A page of its own, named by a nonce. The secret is in neither.
        assert!(
            location.starts_with("/projects/shipping/shown?"),
            "{location}"
        );
        assert!(location.contains("nonce="), "{location}");

        let nonce = location
            .split("nonce=")
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

    /// A node that originates nothing makes no projects.
    ///
    /// The button goes and the POST refuses, and the second is the one
    /// that counts: a form can be submitted by anything, and hiding a
    /// control is a courtesy. The list says why in its own words rather
    /// than showing an empty page — "no projects yet" would read as a
    /// node waiting for its first one.
    #[tokio::test]
    async fn a_node_that_owns_nothing_makes_no_projects() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        crate::network::capability::set_owns(&console.database, false)
            .await
            .expect("set");

        let list = console
            .harness
            .get("/")
            .header("cookie", &admin)
            .send()
            .await
            .body;
        assert!(
            !list.contains("href=\"/projects/new\""),
            "the button is gone: {list}"
        );
        assert!(list.contains("owns nothing"), "and it says why: {list}");

        let refused = console
            .harness
            .post("/projects")
            .header("cookie", &admin)
            .form(&[("name", "shipping")])
            .send()
            .await;
        let location = refused.header("location").unwrap_or_default().to_string();
        assert!(location.contains("error="), "{location}");
        assert!(
            projects::all(&console.database)
                .await
                .expect("projects")
                .is_empty(),
            "and nothing was written"
        );

        // The form itself, hand-typed, goes back to the list rather than
        // rendering a field nothing will accept.
        let typed = console
            .harness
            .get("/projects/new")
            .header("cookie", &admin)
            .send()
            .await;
        assert_eq!(typed.header("location"), Some("/projects"));
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
        let service = crate::repositories::services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("made");
        let replica = crate::repositories::replicas::ensure_here(&console.database, &service.id, 1)
            .await
            .expect("a copy")
            .pop()
            .expect("one");
        crate::repositories::replicas::set_last_error(
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
            // **Except this one, and on purpose.** A minted token is
            // the one thing somebody has to act on right now, so it
            // gets a page with nothing else on it — and the breadcrumb
            // is the way back to the settings that made it.
            ("tokens", vec![("name", "ci")], "/projects/shipping/shown"),
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
            .form(&[("confirm", "doomed")])
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

    /// A deletion nobody confirmed does not happen.
    ///
    /// There was no confirmation at all — one click removed a project and
    /// every service under it. The reasoning recorded for that was that a
    /// dialog needs JavaScript and this console works without it, which is
    /// true of dialogs and not of text fields. Reported by Jorge.
    ///
    /// Posted without the field, and then with the wrong name, because
    /// `required` in the markup is a courtesy the browser extends and a
    /// request can be constructed by hand.
    #[tokio::test]
    async fn a_project_is_not_deleted_without_its_name_typed() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "keepme")])
            .send()
            .await;

        for body in [vec![], vec![("confirm", "something-else")]] {
            console
                .harness
                .post("/projects/keepme/delete")
                .header("cookie", &cookie)
                .form(&body)
                .send()
                .await;
            let list = listing(&console, &cookie).await;
            assert!(
                list.contains("keepme"),
                "the project survived a deletion nobody confirmed"
            );
        }

        // And with the name, it goes.
        console
            .harness
            .post("/projects/keepme/delete")
            .header("cookie", &cookie)
            .form(&[("confirm", "keepme")])
            .send()
            .await;
        assert!(!listing(&console, &cookie).await.contains("keepme"));
    }

    /// The project list as somebody signed in sees it.
    async fn listing(console: &Console, cookie: &str) -> String {
        console
            .harness
            .get("/")
            .header("cookie", cookie)
            .send()
            .await
            .body
    }

    /// The danger zone says what deleting a project does to what is in it.
    ///
    /// It used to read "Nothing is stopped first — do that yourself",
    /// which was true of the code and told somebody to go and do something
    /// the page gave them no way to do. It was also the visible half of a
    /// real fault: the cascade took the service *rows* and nothing stopped
    /// a container or removed a volume, so a deleted project left its
    /// containers running and its data on disk — unreachable for ever,
    /// because a container id is derived from the rows that had just gone.
    ///
    /// Jorge reported the two halves separately: a project deleted with no
    /// word about what happened to what was inside it, and an alert
    /// counting three storage directories nothing claimed. One bug.
    ///
    /// The teardown itself needs containerd, so it is verified on a node.
    /// What a test can hold is the promise on the page — and a promise
    /// that drifts from the code is how the old wording survived.
    #[tokio::test]
    async fn the_danger_zone_says_what_goes_with_a_project() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "doomed")])
            .send()
            .await;

        let page = console
            .harness
            .get("/projects/doomed/settings")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            page.contains("stops and removes every service"),
            "{page:.0}"
        );
        assert!(
            page.contains("its databases stored"),
            "it does not say the databases' data goes"
        );
        assert!(
            !page.contains("do that yourself"),
            "it still tells somebody to stop things the page cannot stop"
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
                .form(&[("confirm", "doomed")])
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
