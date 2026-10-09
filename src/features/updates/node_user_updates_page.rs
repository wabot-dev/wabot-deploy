use hypertext::prelude::*;

use crate::update::github::Release;
use crate::update::notes::{Block, Inline};
use crate::update::runs::{Run, Status};
use crate::views::layouts::layout;
use crate::views::texts::language::t;

pub struct Updates<'a> {
    pub history: &'a [Run],
    pub latest: Option<&'a Run>,
    pub available: &'a Result<crate::update::Availability, crate::update::UpdateError>,
    pub query_error: Option<&'a str>,
    pub query_started: Option<&'a str>,
}

pub fn render<'a>(page: &'a Updates<'a>) -> impl Renderable + 'a {
    let history = page.history;
    let latest = page.latest;
    let available = page.available;
    rsx! {
    (layout::style_tag())
    <h1>(t("Updates"))</h1>
    <p class="tagline">(t("This node installs a release when you ask it to, and never \
         on its own. Installing one restarts the node; the containers \
         on it keep running."))</p>

    @if let Some(message) = &page.query_started {
        <p class="note">(message)</p>
    }
    @if let Some(message) = &page.query_error {
        (layout::error_note(message))
    }

    (live_region(available, latest))

    @if let Ok(available) = &available {
        <h2>(t("Releases"))</h2>
        <div class="stack">
            @for release in &available.releases {
                (release_card(release, available.current))
            }
        </div>
    }

    @if history.len() > 1 {
        <h2>(t("Earlier attempts"))</h2>
        <table>
            <thead>
                <tr><th>(t("Version"))</th><th>(t("When"))</th><th>(t("Outcome"))</th></tr>
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
}

/// The button, as a form: installing is not something a link should do.
pub(crate) fn install_form<'a>(
    release: &'a Release,
    class: &'a str,
    label: &'a str,
) -> impl Renderable + 'a {
    rsx! {
        <form method="post" action="/updates/install">
            <input type="hidden" name="tag" value=(&release.tag)>
            <button class=(class) type="submit">(label)</button>
        </form>
    }
}

/// Where a release stands relative to what is running.
pub(crate) fn state_line(
    release: &Release,
    current: Option<crate::update::github::Version>,
) -> impl Renderable + '_ {
    let installed = current == Some(release.version);
    let older = current
        .map(|current| release.version < current)
        .unwrap_or(false);

    rsx! {
        <dl class="kv">
            <dt>(t("Version"))</dt>
            <dd>(release.version.to_string())</dd>
            <dt>(t("State"))</dt>
            <dd>
                @if installed {
                    (t("running here"))
                } @else if !release.installable() {
                    (t("no build for this machine"))
                } @else if older {
                    (t("older than what is running"))
                } @else {
                    (t("newer than what is running"))
                }
            </dd>
            @if let Some(binary) = &release.binary {
                <dt>(t("Download"))</dt>
                <dd>(&binary.name)(" · ")(megabytes(binary.size))</dd>
            }
        </dl>
    }
}

pub(crate) fn release_card(
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
                        @if release.prerelease { (t(" · pre-release")) }
                    </p>
                </div>
                @if installed {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>(t("Running"))
                    </span>
                } @else if !release.installable() {
                    <span class="badge">(t("No build here"))</span>
                } @else {
                    (install_form(release, "btn btn-secondary btn-sm", "Install"))
                }
            </div>
        </section>
    }
}

/// The parts of this page that change without anybody clicking.
///
/// One island around both cards, because an update changes both: the
/// badge while it runs, and the **running version** when it is over.
///
/// This is the one page where a stream earns its keep twice. An update
/// ends by replacing this process, so the socket does not merely go
/// quiet — it dies, and an `EventSource` reconnects on its own when the
/// new binary is listening. The page can then say "the node is
/// restarting" while it is gone and fill in the version that came back,
/// which is exactly what "reload this page in a few seconds" was asking
/// a person to do by hand.
pub(crate) fn live_region<'a>(
    available: &'a crate::update::UpdateResult<crate::update::Availability>,
    latest: Option<&'a Run>,
) -> impl Renderable + 'a {
    let inner = rsx! {
        <section class="card stack">
            <div class="split">
                <div class="stack-sm">
                    <p class="card-label">(t("Running"))</p>
                    <p class="mono" data-run="version">
                        ("wabot-deploy ")(crate::api::VERSION)
                    </p>
                </div>
                <form method="post" action="/updates/check">
                    <button class="btn btn-secondary btn-sm" type="submit">
                        (t("Check again"))
                    </button>
                </form>
            </div>

            @match available {
                Err(error) => {
                    <p class="failure">
                        (t("Could not read the release list: "))(error.to_string())
                    </p>
                }
                Ok(available) => {
                    @if let Some(release) = &available.upgrade {
                        <p>(&release.name)(t(" is available."))</p>
                        (install_form(release, "btn", "Install this release"))
                    } @else {
                        <p class="note">(t("This is the newest release published."))</p>
                    }
                }
            }
        </section>

        @if let Some(run) = latest {
            (run_card(run))
        }
    }
    .render()
    .into_inner();

    wabot::ui::hypertext::island(
        "updates-live",
        &serde_json::json!({}),
        hypertext::Raw::dangerously_create(&inner),
    )
}

/// `is-hidden`, or nothing.
///
/// The server renders both halves and a class decides, so the stream
/// never has to build markup — the island rule the whole console
/// follows.
pub(crate) fn hidden_unless(shown: bool) -> &'static str {
    match shown {
        true => "",
        false => "is-hidden",
    }
}

/// What the last attempt is doing, or did.
pub(crate) fn run_card(run: &Run) -> impl Renderable + '_ {
    rsx! {
        <section class="card card-sunken stack-sm">
            <div class="split">
                <p class="card-label">(t("Last update"))</p>
                <span data-run="badge">(status_badge(run.status))</span>
            </div>
            <dl class="kv">
                <dt>(t("Version"))</dt>
                <dd>(&run.from_version)(" → ")(&run.to_version)</dd>
                <dt>(t("Started"))</dt>
                <dd>(layout::when(run.started_at))</dd>
                @if let Some(finished) = run.finished_at {
                    <dt>(t("Finished"))</dt>
                    <dd>(layout::when(finished))</dd>
                }
                // Always rendered, so the stream has somewhere to
                // write. Hidden while there is nothing to say rather
                // than absent — markup that appears is markup the
                // client would have to build.
                <dt data-run="step-label"
                    class=(hidden_unless(run.status.in_flight()))>(t("Now"))</dt>
                <dd data-run="step"
                    class=(hidden_unless(run.status.in_flight()))>(
                    run.step.as_deref().unwrap_or_default()
                )</dd>
                @if let Some(backup) = &run.backup_path {
                    <dt>(t("Backup"))</dt>
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
            <p data-run="waiting"
               class=(format!("note {}", hidden_unless(run.status.in_flight())))>(t("The node restarts on its own when the new binary is in place. \
                 This page follows along — there is nothing to reload."))</p>
        </section>
    }
}

pub(crate) fn status_badge(status: Status) -> impl Renderable {
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
pub(crate) fn rendered_notes(markdown: &str) -> impl Renderable + '_ {
    let blocks = crate::update::notes::parse(markdown);
    rsx! {
        @if blocks.is_empty() {
            <p class="note">(t("This release came with no notes."))</p>
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

pub(crate) fn inlines_of(inlines: &[Inline]) -> impl Renderable + '_ {
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
pub(crate) fn day_of(published_at: &str) -> &str {
    published_at.split('T').next().unwrap_or(published_at)
}

pub(crate) fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timestamp_becomes_a_day() {
        assert_eq!(day_of("2026-08-07T10:00:00Z"), "2026-08-07");
        assert_eq!(day_of("whenever"), "whenever");
    }
}
