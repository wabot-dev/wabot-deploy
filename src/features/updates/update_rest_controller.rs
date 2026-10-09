//! What the updates page posts to, and the stream it feeds.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::infra::update::runs::Status;

use crate::web::console::auth::{signed_in, SessionMiddleware};
use crate::web::console::ConsoleState;

/// The server half.
#[injectable]
pub struct UpdateRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl UpdateRestController {
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
            .is_some_and(|account| crate::domain::authz::node_user::manages_the_machine(&account))
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
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }

        match crate::features::updates::node_user_check_for_updates::run(
            &self.state.catalogue,
            &account,
        )
        .await
        {
            Ok(()) => Ok(crate::web::console::auth::see_other("/updates")),
            Err(reason) => Ok(crate::web::console::auth::back_with_error(
                "/updates", &reason,
            )),
        }
    }

    /// Start an update. Answers immediately; the work outlives the
    /// request and ends by replacing this process.
    #[post("/updates/install")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn install(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::web::console::auth::see_other("/sign-in"));
        };
        if !crate::domain::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::web::console::auth::see_other("/"));
        }
        let form = match crate::web::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let started = match crate::features::updates::node_user_install_release::run(
            &self.state.database,
            &self.state.config,
            &self.state.catalogue,
            &account,
            crate::web::console::auth::field(&form, "tag"),
        )
        .await
        {
            Ok(started) => started,
            Err(reason) => {
                return Ok(crate::web::console::auth::back_with_error(
                    "/updates", &reason,
                ))
            }
        };

        Ok(crate::web::console::auth::see_other(&format!(
            "/updates?{}",
            crate::web::console::auth::encode(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::update::runs;
    use crate::web::console::tests::Console;
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

        assert!(
            body.contains(crate::infra::update::VERSION),
            "says what is running"
        );
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

async fn live_cells(state: &crate::web::console::ConsoleState) -> LiveRun {
    let Ok(Some(run)) = crate::infra::update::runs::latest(&state.database).await else {
        return LiveRun {
            none: true,
            version: crate::infra::update::VERSION,
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
        version: crate::infra::update::VERSION,
        none: false,
    }
}
