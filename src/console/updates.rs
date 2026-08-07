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
use crate::update::github::Release;
use crate::update::notes::{Block, Inline};
use crate::update::runs::{self, Run, Status};

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
        if !account.is_admin() {
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
        let frame = Frame::new(&account, Area::Nodes, &projects, None, "/updates");
        let body = rsx! {
            (layout::style_tag())
            <h1>("Updates")</h1>
            <p class="tagline">(
                "This node installs a release when you ask it to, and never \
                 on its own. Installing one restarts the node; the containers \
                 on it keep running."
            )</p>

            @if let Some(message) = &query.started {
                <p class="note">(message)</p>
            }
            @if let Some(message) = &query.error {
                (layout::error_note(message))
            }

            <section class="card stack">
                <div class="split">
                    <div class="stack-sm">
                        <p class="card-label">("Running")</p>
                        <p class="mono">("wabot-deploy ")(crate::api::VERSION)</p>
                    </div>
                    <form method="post" action="/updates/check">
                        <button class="btn btn-secondary btn-sm" type="submit">
                            ("Check again")
                        </button>
                    </form>
                </div>

                @match &available {
                    Err(error) => {
                        <p class="failure">
                            ("Could not read the release list: ")(error.to_string())
                        </p>
                    }
                    Ok(available) => {
                        @if let Some(release) = &available.upgrade {
                            <p>
                                (&release.name)(" is available.")
                            </p>
                            (install_form(release, "btn", "Install this release"))
                        } @else {
                            <p class="note">("This is the newest release published.")</p>
                        }
                    }
                }
            </section>

            @if let Some(run) = &latest {
                (run_card(run))
            }

            @if let Ok(available) = &available {
                <h2>("Releases")</h2>
                <div class="stack">
                    @for release in &available.releases {
                        (release_card(release, available.current))
                    }
                </div>
            }

            @if history.len() > 1 {
                <h2>("Earlier attempts")</h2>
                <table>
                    <thead>
                        <tr><th>("Version")</th><th>("When")</th><th>("Outcome")</th></tr>
                    </thead>
                    <tbody>
                        @for run in history.iter().skip(1) {
                            <tr>
                                <td class="mono">(&run.to_version)</td>
                                <td>(layout::when(run.started_at))</td>
                                <td>
                                    (status_badge(run.status))
                                    @if let Some(detail) = &run.detail {
                                        <span class="tile-detail"> (detail)</span>
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            }
        }
        .render()
        .into_inner();

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
        if !account.is_admin() {
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
        let frame = Frame::new(&account, Area::Nodes, &projects, None, "/updates");
        let body = rsx! {
            (layout::style_tag())
            <p class="crumb"><a href="/updates">("Updates")</a></p>
            <h1>(&release.name)</h1>
            <p class="tagline">
                (&release.tag)
                @if !release.published_at.is_empty() {
                    (" · published ")(day_of(&release.published_at))
                }
            </p>

            @if let Some(message) = &query.error {
                (layout::error_note(message))
            }

            <section class="card stack">
                (state_line(&release, current))
                @if release.installable() && current != Some(release.version) {
                    (install_form(&release, "btn", "Install this release"))
                }
            </section>

            <section class="card stack">
                <p class="card-label">("What is in it")</p>
                (rendered_notes(&release.notes))
                @if !release.html_url.is_empty() {
                    <p class="tile-detail">
                        <a href=(&release.html_url) rel="noreferrer noopener" target="_blank">
                            ("Read it on GitHub")
                        </a>
                    </p>
                }
            </section>
        }
        .render()
        .into_inner();

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
    /// Ask GitHub again, rather than explaining the cache to somebody
    /// who just published a release.
    #[post("/updates/check")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn check(&self, _request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !account.is_admin() {
            return Ok(super::auth::see_other("/"));
        }

        self.state.catalogue.refresh().await;
        Ok(super::auth::see_other("/updates"))
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
        if !account.is_admin() {
            return Ok(super::auth::see_other("/"));
        }

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let tag = super::auth::field(&form, "tag").to_string();
        if tag.is_empty() {
            return Ok(super::auth::back_with_error(
                "/updates",
                "no release chosen",
            ));
        }

        // Everything that can be refused, refused here — while there
        // is still a request to answer. What happens after this point
        // is reported on the run row, because the browser that started
        // it is about to lose the server it asked.
        let releases = match self.state.catalogue.releases().await {
            Ok(releases) => releases,
            Err(error) => return Ok(super::auth::back_with_error("/updates", &error.to_string())),
        };
        let Some(release) = crate::update::github::find(&releases, &tag) else {
            return Ok(super::auth::back_with_error(
                "/updates",
                &format!("no release is tagged {tag}"),
            ));
        };
        if !release.installable() {
            return Ok(super::auth::back_with_error(
                "/updates",
                &format!("{} published no build this node can install", release.tag),
            ));
        }
        if crate::update::github::Version::current() == Some(release.version) {
            return Ok(super::auth::back_with_error(
                "/updates",
                &format!("{} is what this node is already running", release.tag),
            ));
        }
        let in_flight = match runs::in_flight(&self.state.database).await {
            Ok(existing) => existing,
            Err(error) => return Ok(super::auth::back_with_error("/updates", &error.to_string())),
        };
        if let Some(existing) = in_flight {
            return Ok(super::auth::back_with_error(
                "/updates",
                &format!("an update to {} is already running", existing.to_version),
            ));
        }
        if !crate::bootstrap::service::supervised() {
            return Ok(super::auth::back_with_error(
                "/updates",
                "nothing supervises this node, so it cannot restart itself",
            ));
        }

        crate::update::start_in_background(
            self.state.database.clone(),
            self.state.config.clone(),
            self.state.catalogue.clone(),
            release.tag.clone(),
            Some(account.id.clone()),
        );

        Ok(super::auth::see_other(&format!(
            "/updates?{}",
            encode(
                "started",
                &format!(
                    "Installing {}. The node restarts when it is ready — this page \
                     will be unreachable for a few seconds, then reload it.",
                    release.tag
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

/// The button, as a form: installing is not something a link should do.
fn install_form<'a>(release: &'a Release, class: &'a str, label: &'a str) -> impl Renderable + 'a {
    rsx! {
        <form method="post" action="/updates/install">
            <input type="hidden" name="tag" value=(&release.tag)>
            <button class=(class) type="submit">(label)</button>
        </form>
    }
}

/// Where a release stands relative to what is running.
fn state_line(
    release: &Release,
    current: Option<crate::update::github::Version>,
) -> impl Renderable + '_ {
    let installed = current == Some(release.version);
    let older = current
        .map(|current| release.version < current)
        .unwrap_or(false);

    rsx! {
        <dl class="kv">
            <dt>("Version")</dt>
            <dd>(release.version.to_string())</dd>
            <dt>("State")</dt>
            <dd>
                @if installed {
                    ("running here")
                } @else if !release.installable() {
                    ("no build for this machine")
                } @else if older {
                    ("older than what is running")
                } @else {
                    ("newer than what is running")
                }
            </dd>
            @if let Some(binary) = &release.binary {
                <dt>("Download")</dt>
                <dd>(&binary.name)(" · ")(megabytes(binary.size))</dd>
            }
        </dl>
    }
}

fn release_card(
    release: &Release,
    current: Option<crate::update::github::Version>,
) -> impl Renderable + '_ {
    let installed = current == Some(release.version);
    rsx! {
        <section class="card stack-sm">
            <div class="split">
                <div class="stack-sm">
                    <p class="tile-name">
                        <a href=(format!("/updates/{}", release.tag))>(&release.name)</a>
                    </p>
                    <p class="tile-detail">
                        (&release.tag)
                        @if !release.published_at.is_empty() {
                            (" · ")(day_of(&release.published_at))
                        }
                        @if release.prerelease { (" · pre-release") }
                    </p>
                </div>
                @if installed {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>("Running")
                    </span>
                } @else if !release.installable() {
                    <span class="badge">("No build here")</span>
                } @else {
                    (install_form(release, "btn btn-secondary btn-sm", "Install"))
                }
            </div>
        </section>
    }
}

/// What the last attempt is doing, or did.
fn run_card(run: &Run) -> impl Renderable + '_ {
    rsx! {
        <section class="card card-sunken stack-sm">
            <div class="split">
                <p class="card-label">("Last update")</p>
                (status_badge(run.status))
            </div>
            <dl class="kv">
                <dt>("Version")</dt>
                <dd>(&run.from_version)(" → ")(&run.to_version)</dd>
                <dt>("Started")</dt>
                <dd>(layout::when(run.started_at))</dd>
                @if let Some(finished) = run.finished_at {
                    <dt>("Finished")</dt>
                    <dd>(layout::when(finished))</dd>
                }
                @if let Some(step) = &run.step {
                    @if run.status.in_flight() {
                        <dt>("Now")</dt>
                        <dd>(step)</dd>
                    }
                }
                @if let Some(backup) = &run.backup_path {
                    <dt>("Backup")</dt>
                    <dd>(backup)</dd>
                }
            </dl>
            @if let Some(detail) = &run.detail {
                @if run.status == Status::Failed {
                    <p class="failure">(detail)</p>
                } @else {
                    <p class="note">(detail)</p>
                }
            }
            @if run.status.in_flight() {
                <p class="note">(
                    "The node restarts on its own when the new binary is in place. \
                     Reload this page in a few seconds."
                )</p>
            }
        </section>
    }
}

fn status_badge(status: Status) -> impl Renderable {
    let (class, dot, label) = match status {
        Status::Done => ("badge badge-success", "dot dot-success", "Installed"),
        Status::Failed => ("badge badge-danger", "dot dot-danger", "Failed"),
        Status::Restarting => ("badge badge-info", "dot dot-info dot-pulse", "Restarting"),
        Status::Running => ("badge badge-info", "dot dot-info dot-pulse", "Installing"),
    };
    rsx! {
        <span class=(class)><span class=(dot)></span>(label)</span>
    }
}

/// Release notes, rendered from the structure [`crate::update::notes`]
/// parsed. Every value goes through `rsx!`, which escapes it.
fn rendered_notes(markdown: &str) -> impl Renderable + '_ {
    let blocks = crate::update::notes::parse(markdown);
    rsx! {
        @if blocks.is_empty() {
            <p class="note">("This release came with no notes.")</p>
        }
        <div class="notes">
            @for block in &blocks {
                @match block {
                    Block::Heading(inlines) => { <h3>(inlines_of(inlines))</h3> }
                    Block::Paragraph(inlines) => { <p>(inlines_of(inlines))</p> }
                    Block::Code(text) => { <pre><code>(text)</code></pre> }
                    Block::List(items) => {
                        <ul>
                            @for item in items {
                                <li>(inlines_of(item))</li>
                            }
                        </ul>
                    }
                }
            }
        </div>
    }
}

fn inlines_of(inlines: &[Inline]) -> impl Renderable + '_ {
    rsx! {
        @for inline in inlines {
            @match inline {
                Inline::Text(text) => { (text) }
                Inline::Code(text) => { <code>(text)</code> }
                Inline::Link { text, url } => {
                    <a href=(url) rel="noreferrer noopener" target="_blank">(text)</a>
                }
            }
        }
    }
}

/// `2026-08-07T10:00:00Z` → `2026-08-07`. GitHub's timestamp is
/// already sorted and already unambiguous; the clock time is noise on
/// a page about which version to install.
fn day_of(published_at: &str) -> &str {
    published_at.split('T').next().unwrap_or(published_at)
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    #[test]
    fn a_timestamp_becomes_a_day() {
        assert_eq!(day_of("2026-08-07T10:00:00Z"), "2026-08-07");
        assert_eq!(day_of("whenever"), "whenever");
    }

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
