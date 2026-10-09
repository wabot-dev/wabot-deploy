use hypertext::prelude::*;

use crate::features::services::service_push_component::urlencode;
use crate::infra::deploy::logs::{Found, Line};
use crate::web::views::layouts::layout;
use crate::web::views::texts::language::t;

/// How many matches one page of results holds. Here rather than in the
/// controller: it is what decides whether the older/newer links appear,
/// and the number that pages the query has to be the number that draws
/// them.
pub const PER_PAGE: usize = 50;

pub struct Logs<'a> {
    pub project_slug: &'a str,
    pub service_slug: &'a str,
    pub service_name: &'a str,

    /// The copies with a log that can be read at all, and the one being
    /// read. `slot` is `None` only when there is nothing to read.
    pub readable: &'a [u32],
    pub slot: Option<u32>,
    /// The node running the chosen copy, when it is not this one.
    /// **Every reading branches on it**: a log fetched over the network
    /// has no offset to follow from, nothing to search locally, and no
    /// generations this node can count.
    pub holder: Option<&'a str>,
    /// Every copy is somewhere else and none has said where yet.
    pub elsewhere: bool,

    /// The whole kept history rather than the live tail.
    pub everything: bool,
    /// There is older history to offer, so the link is absent when it
    /// would lead to the same bytes.
    pub has_history: bool,
    /// **Whether there is a file at all**, which is not whether it has
    /// anything in it. Three states, not two: a container started before
    /// its output was being kept has no file, and telling that one
    /// "nothing yet" is the page saying the container is quiet when the
    /// truth is that nobody was listening.
    pub kept: bool,
    pub text: &'a str,
    /// Where the island resumes from. Zero for a reading that came over
    /// the network: a number counted there is not a place in anything
    /// here.
    pub from: u64,
    /// The far node's own words. Never paraphrased — this node cannot
    /// see that machine.
    pub remote_error: Option<&'a str>,

    pub needle: &'a str,
    pub page: usize,
    pub found: Option<&'a Found>,
    /// The line being read in context, and the lines around it.
    pub at: Option<usize>,
    pub window: &'a [Line],
}

impl Logs<'_> {
    fn logs_path(&self) -> String {
        format!(
            "/projects/{}/services/{}/logs",
            self.project_slug, self.service_slug
        )
    }
}

pub fn render<'a>(page: &'a Logs<'a>) -> impl Renderable + 'a {
    let path = page.logs_path();
    rsx! {
            (layout::style_tag())
            // Title and copy tabs on one line, the shape every other page
            // here uses for a heading and its controls — and the tabs belong
            // beside it rather than in a row of their own, because they
            // select what the panel below shows.
            <div class="split">
                <div class="stack-sm">
                    <h1>(page.service_name)</h1>
                    // **This said the opposite of what happens.** It
                    // described `prepare`, which truncated on every
                    // deployment, and `resume` replaced that precisely
                    // because a deployment is the moment somebody most wants
                    // what came before it. Nothing updated the sentence, so
                    // the page promised no history on the page that has one.
                    // Caught by Jorge reading it.
                    <p class="tile-detail">(t("What this copy has written. A deployment marks \
                         a line and carries on rather than starting the file again, so this \
                         holds every run since the last rotation — and the ones before that \
                         are in everything kept."))</p>
                </div>
                @if page.readable.len() > 1 {
                    <div class="row">
                        @for one in page.readable {
                            @if Some(*one) == page.slot {
                                <span class="badge">(t("Copy "))(one)</span>
                            } @else {
                                <a class="btn btn-ghost btn-sm"
                                   href=(format!("{path}?slot={one}"))>(t("Copy "))(one)</a>
                            }
                        }
                    </div>
                }
            </div>

            @if let Some(slot) = page.slot {
                <section class="card stack">
                    <div class="split">
                        @if page.everything {
                            <p class="card-label">(t("Everything kept"))</p>
                        } @else {
                            <p class="card-label">(t("Output"))</p>
                        }
                        @if page.holder.is_some() {
                            // A copy on another machine does not follow, and
                            // for a plainer reason: this is one answer to one
                            // question, fetched when the page was rendered.
                            // Reloading asks again.
                            <span class="tile-detail">
                                (t("Read from the node running it — reload to ask again"))
                            </span>
                        } @else if page.everything {
                            // The whole view does not follow: its offsets are
                            // into a concatenation, so resuming from one would
                            // read the wrong bytes. Said plainly rather than
                            // leaving a stream label that lies.
                            <a class="btn btn-ghost btn-sm"
                               href=(format!("{path}?slot={slot}"))>(t("Follow what it is saying now"))</a>
                        } @else {
                            // Written by the island, and rendered whether or
                            // not scripting is on: a label that only appears
                            // with a script is one somebody without a script
                            // cannot read.
                            <span class="tile-detail" data-logs-state>
                                (t("Not following — reload to see more"))
                            </span>
                        }
                    </div>
                    @if (page.has_history || page.holder.is_some()) && !page.everything {
                        <p class="tile-detail">
                            (t("Older output is kept from before the last restarts."))
                            " "
                            <a href=(format!("{path}?slot={slot}&all=1"))>(t("Show everything kept"))</a>
                        </p>
                    }
                    @if let Some(reason) = page.remote_error {
                        (layout::error_note(reason))
                    }
                    // **Searching is a link, not a script.** A GET form, so a
                    // search can be shared, bookmarked and gone back from —
                    // and it works with scripting off, which is the rule this
                    // console is built to. `slot` rides along or a search
                    // would jump to another copy.
                    //
                    // Only for a copy here. Searching a log on another
                    // machine means that machine scanning it, which is a
                    // different endpoint and a decision about somebody else's
                    // one core — so the reading travels and the sweep does
                    // not.
                    @if page.holder.is_none() {
                        <form method="get" class="row" action=(&path)>
                            <input type="hidden" name="slot" value=(slot)>
                            <input name="q" type="search" autocomplete="off" class="mono"
                                   value=(page.needle)
                                   placeholder=(t("Find in everything kept"))>
                            <button class="btn btn-secondary" type="submit">(t("Find"))</button>
                        </form>
                    }

                    @if let Some(found) = page.found {
                        @if found.total == 0 {
                            <p class="tile-detail">(t("Nothing matched, in "))
                                (found.scanned)(t(" lines kept."))</p>
                        } @else {
                            <p class="tile-detail">
                                (found.total)(t(" matched, of "))(found.scanned)
                                (t(" lines kept."))
                            </p>
                            // Every match a link to itself in context. The
                            // number is what it is counted at, and the page
                            // says so: a sweep that drops the oldest
                            // generation shifts them all down, so this is a
                            // place and not an address.
                            <table>
                                <tbody>
                                    @for line in &found.lines {
                                        <tr>
                                            <td class="mono tile-detail">
                                                <a href=(format!(
                                                    "{path}?slot={slot}&q={}&page={}&at={}",
                                                    urlencode(page.needle), page.page, line.number
                                                ))>(line.number)</a>
                                            </td>
                                            <td class="mono log-hit">(&line.text)</td>
                                        </tr>
                                    }
                                </tbody>
                            </table>
                            @if found.total > PER_PAGE {
                                <div class="row">
                                    @if page.page > 1 {
                                        <a class="btn btn-ghost btn-sm" href=(format!(
                                            "{path}?slot={slot}&q={}&page={}",
                                            urlencode(page.needle), page.page - 1
                                        ))>(t("Newer matches"))</a>
                                    }
                                    @if page.page * PER_PAGE < found.total {
                                        <a class="btn btn-ghost btn-sm" href=(format!(
                                            "{path}?slot={slot}&q={}&page={}",
                                            urlencode(page.needle), page.page + 1
                                        ))>(t("Older matches"))</a>
                                    }
                                </div>
                            }
                        }
                        @if !page.window.is_empty() {
                            @let target = page.at.unwrap_or(0);
                            <p class="card-label">(t("Around line "))(target)</p>
                            <pre class="log">@for line in page.window {
    @if line.number == target {
    (">")(" ")(&line.text)("\n")
    } @else {
    ("  ")(&line.text)("\n")
    }
    }</pre>
                        }
                    } @else if page.holder.is_some() {
                        // No `data-logs-out`: the island polls this node's own
                        // endpoint, and there is nothing of this copy here to
                        // poll for.
                        <pre class="log">(page.text)</pre>
                    } @else {
                        <pre class="log" data-logs-out
                             data-slot=(slot)
                             data-from=(page.from)>(page.text)</pre>
                    }
                    @if !page.kept && page.holder.is_some() {
                        // The reason is already above, in the far node's own
                        // words. Repeating a guess under it would be this node
                        // explaining a machine it cannot see.
                    } @else if !page.kept {
                        <p class="tile-detail">(t("This container was started before its output \
                             was being kept. Deploy it again and it will \
                             write from then on."))</p>
                    } @else if page.text.trim().is_empty() {
                        <p class="tile-detail">(t("Nothing yet. A container that has only just \
                             started may not have written anything."))</p>
                    }
                </section>
            } @else if page.elsewhere {
                // Reached when every copy is elsewhere *and* none of them has
                // a node recorded — a placement that has not been reported
                // yet. Once one has, its slot is readable and the panel above
                // is what renders.
                <section class="card stack">
                    <p>(t("No copy of this service runs on this node."))</p>
                    <p class="tile-detail">(t("A copy writes its log on the machine that runs it. This \
                         node asks that machine for it — but it has not said which \
                         machine yet, which it does on its next report."))</p>
                </section>
            } @else {
                <section class="card stack">
                    <p>(t("This service is not running anywhere."))</p>
                </section>
            }
        }
}
