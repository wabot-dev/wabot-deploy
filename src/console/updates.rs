//! Updates: what this node is running, what it could run, and the
//! button that changes it.
//!
//! ## One click, and everything it has to say first
//!
//! Installing a release restarts the node — every container keeps
//! running, but the console, the proxy and the registry go away for a
//! few seconds. So the page shows what is about to happen before it
//! offers the button: which version, published when, and the notes
//! that came with it.
//!
//! ## What the page reads while an update runs
//!
//! The run row, which the update writes as it goes. The process is
//! replaced at the end, so the page a browser reloads afterwards is
//! served by the *new* binary reading a row the old one wrote — which
//! is exactly why that state lives in the database.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use crate::platform::access;
use crate::update::runs::{self, Status};

use super::auth::{signed_in, SessionMiddleware};
use super::shell::{Area, Frame};
use super::{layout, ConsoleState};

#[derive(Debug, Deserialize, Validate)]
pub struct UpdatesPage {
    pub error: Option<String>,
    pub started: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ReleasePage {
    pub tag: String,
    pub error: Option<String>,
}

#[injectable]
pub struct UpdatePages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl UpdatePages {
    #[view("/updates")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: UpdatesPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        // Updating the node is the node's business, which belongs to
        // whoever runs it.
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        let projects = access::projects_for(&self.state.database, &account).await?;
        let history = runs::recent(&self.state.database, 5)
            .await
            .unwrap_or_else(|error| {
                // The release list and the version are the page; a
                // history that cannot be read is a missing card, not a
                // missing page.
                tracing::warn!(%error, "could not read the update history");
                Vec::new()
            });
        let latest = history.first().cloned();

        // A failure to reach GitHub is not a failure of the page: this
        // node runs fine without ever asking, and the page still has
        // to say what version it is and what happened last time.
        let available = crate::update::availability(&self.state.catalogue).await;

        layout::head("Updates");
        let frame = Frame::new(&account, Area::Settings, &projects, None, "/updates");
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::pages::node_user_updates_page::Updates {
            history: &history,
            latest: latest.as_ref(),
            available: &available,
            query_error: query.error.as_deref(),
            query_started: query.started.as_deref(),
        };
        let body = super::language::scoped(account.language, || {
            crate::pages::node_user_updates_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// One release, in full — the notes are the reason this page
    /// exists rather than a version number on a button.
    #[view("/updates/:tag")]
    #[middleware(SessionMiddleware)]
    async fn release(&self, query: ReleasePage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        let releases = match self.state.catalogue.releases().await {
            Ok(releases) => releases,
            Err(error) => {
                let back = format!("/updates?{}", encode("error", &error.to_string()));
                return Ok(Redirect::found(back).into());
            }
        };
        let Some(release) = crate::update::github::find(&releases, &query.tag).cloned() else {
            return Ok(Redirect::found("/updates").into());
        };

        let projects = access::projects_for(&self.state.database, &account).await?;
        let current = crate::update::github::Version::current();

        layout::head(&format!("wabot-deploy {}", release.version));
        let frame = Frame::new(&account, Area::Settings, &projects, None, "/updates");
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::pages::node_user_release_page::ReleaseView {
            release: &release,
            current,
            query_error: query.error.as_deref(),
        };
        let body = super::language::scoped(account.language, || {
            crate::pages::node_user_release_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}

/// The server half.
#[injectable]
pub struct UpdateApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl UpdateApi {
    /// What the update is doing, until it takes the process with it.
    ///
    /// The one stream here whose *disconnection* carries meaning. See
    /// [`live_region`].
    #[get("/updates/live")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn live(&self, _request: Request) -> RestResult<Response> {
        use wabot::rest::axum::body::{Body, Bytes};
        use wabot::rest::axum::http::{header, StatusCode};

        if !signed_in(&self.auth)
            .is_some_and(|account| crate::authz::node_user::manages_the_machine(&account))
        {
            // A status rather than a redirect: an `EventSource` cannot
            // follow one usefully.
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        }

        let state = self.state.clone();
        let stream = async_stream::stream! {
            loop {
                let payload = serde_json::to_string(&live_cells(&state).await)
                    .unwrap_or_else(|_| "{}".into());
                yield Ok::<_, std::convert::Infallible>(
                    Bytes::from(format!("data: {payload}\n\n")),
                );
                // A second, not two: an install moves through its steps
                // quickly and this is the page somebody is watching
                // precisely because they cannot do anything else.
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
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

    /// Ask GitHub again, rather than explaining the cache to somebody
    /// who just published a release.
    #[post("/updates/check")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn check(&self, _request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }

        match crate::usecases::node_user_check_for_updates::run(&self.state.catalogue, &account)
            .await
        {
            Ok(()) => Ok(super::auth::see_other("/updates")),
            Err(reason) => Ok(super::auth::back_with_error("/updates", &reason)),
        }
    }

    /// Start an update. Answers immediately; the work outlives the
    /// request and ends by replacing this process.
    #[post("/updates/install")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn install(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let started = match crate::usecases::node_user_install_release::run(
            &self.state.database,
            &self.state.config,
            &self.state.catalogue,
            &account,
            super::auth::field(&form, "tag"),
        )
        .await
        {
            Ok(started) => started,
            Err(reason) => return Ok(super::auth::back_with_error("/updates", &reason)),
        };

        Ok(super::auth::see_other(&format!(
            "/updates?{}",
            encode(
                "started",
                &format!(
                    "Installing {}. The node restarts when it is ready — this page \
                     will be unreachable for a few seconds, then reload it.",
                    started.tag
                )
            )
        )))
    }
}

fn encode(key: &str, value: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair(key, value)
        .finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    /// The page has to render on a node that cannot reach GitHub — the
    /// version it is running and what happened last time are local
    /// facts, and a network failure must not take them away.
    #[tokio::test]
    async fn the_page_renders_without_the_release_list() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/updates")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(body.contains(crate::api::VERSION), "says what is running");
        assert!(body.contains("Updates"), "{body}");
    }

    #[tokio::test]
    async fn only_an_admin_sees_it() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        let response = console
            .harness
            .get("/updates")
            .header("cookie", &member)
            .send()
            .await;
        response.assert_status(StatusCode::FOUND);
        assert_eq!(response.header("location"), Some("/"));
    }

    /// Installing is a POST, and a member posting it must not start an
    /// update on somebody else's node.
    #[tokio::test]
    async fn only_an_admin_may_install() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        let response = console
            .harness
            .post("/updates/install")
            .header("cookie", &member)
            .form(&[("tag", "v9.9.9")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
        assert!(runs::latest(&console.database)
            .await
            .expect("read")
            .is_none());
    }

    /// A request with no release named must not reach the part that
    /// downloads things.
    #[tokio::test]
    async fn installing_nothing_is_an_error_not_an_update() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .post("/updates/install")
            .header("cookie", &cookie)
            .form(&[("tag", "")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").unwrap_or_default();
        assert!(location.starts_with("/updates?error="), "{location}");
        assert!(runs::latest(&console.database)
            .await
            .expect("read")
            .is_none());
    }

    /// The history is local: it renders whatever the network says.
    #[tokio::test]
    async fn a_finished_run_is_reported_on_the_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let run = runs::start(&console.database, "0.1.0", "0.2.0", "v0.2.0", None)
            .await
            .expect("start");
        runs::finish(
            &console.database,
            &run.id,
            Status::Failed,
            Some("the download does not match the published checksum"),
        )
        .await
        .expect("finish");

        let body = console
            .harness
            .get("/updates")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(body.contains("Failed"), "{body}");
        assert!(
            body.contains("does not match the published checksum"),
            "{body}"
        );
    }
}

/// What the update page writes where.
///
/// `version` matters as much as the badge: the same figure that renders
/// on load, so a page that reconnected after the restart can say
/// *which* binary answered rather than only that something did.
#[derive(serde::Serialize, Default)]
struct LiveRun {
    badge: &'static str,
    dot: &'static str,
    word: &'static str,
    step: String,
    /// Whether it is still going, which decides what the page hides.
    in_flight: bool,
    version: &'static str,
    /// No run at all, on a node that has never been updated. The page
    /// renders no card, so saying so keeps the client from writing into
    /// markup that is not there.
    none: bool,
}

async fn live_cells(state: &super::ConsoleState) -> LiveRun {
    let Ok(Some(run)) = crate::update::runs::latest(&state.database).await else {
        return LiveRun {
            none: true,
            version: crate::api::VERSION,
            ..Default::default()
        };
    };

    let (badge, dot, word) = match run.status {
        Status::Done => ("badge badge-success", "dot dot-success", "Installed"),
        Status::Failed => ("badge badge-danger", "dot dot-danger", "Failed"),
        Status::Restarting => ("badge badge-info", "dot dot-info dot-pulse", "Restarting"),
        Status::Running => ("badge badge-info", "dot dot-info dot-pulse", "Installing"),
    };

    LiveRun {
        badge,
        dot,
        word,
        step: run.step.clone().unwrap_or_default(),
        in_flight: run.status.in_flight(),
        version: crate::api::VERSION,
        none: false,
    }
}
