//! Where this node's backups go, how often, and what the archive holds.

use crate::features::backups::texts::t;
use crate::web::views::layouts::layout;
use hypertext::prelude::*;

/// Where a copy of this node goes, and when.
///
/// One form, because it is one decision: a destination with no schedule
/// is a field nobody acts on, and a schedule with nowhere to write is a
/// job that fails at three in the morning. The write-ahead log switch is
/// here too — it decides how far a restore *reaches* from whatever this
/// takes, and it was two tabs away from the thing it depends on.
pub fn backup_card<'a>(
    plan: &'a crate::infra::backup::plan::Plan,
    archiving: bool,
    default_destination: &'a str,
) -> impl Renderable + 'a {
    use crate::infra::backup::plan::Cadence;

    let destination = plan.destination.clone().unwrap_or_default();
    let keep_days = plan.keep_days.to_string();
    // Written out rather than taken from a table so that the scan in
    // `es.rs` can see them: it reads the source for calls to `t` with a
    // literal in them, and a label that arrives as a value is a string it
    // cannot check.
    let days = [
        t("Monday"),
        t("Tuesday"),
        t("Wednesday"),
        t("Thursday"),
        t("Friday"),
        t("Saturday"),
        t("Sunday"),
    ];

    // An island host for the same reason the certificate form is one:
    // what the runtime hides here is only what does not apply, and with
    // scripting off every field is visible and the form still submits.
    wabot::ui::hypertext::island_bare(
        "fields",
        rsx! {
            <section class="stack" id="backup">
                <p class="card-label">(t("Backing up this node"))</p>
                <form method="post" action="/node/backups" class="card stack">
                    <label for="backup-destination">(t("Where the copy goes"))</label>
                    // The path is a path: not translated, and monospaced
                    // like every other thing on this console that somebody
                    // pastes into a terminal.
                    <input id="backup-destination" name="destination" type="text"
                           autocomplete="off" class="mono" value=(&destination)
                           placeholder=(default_destination)>
                    <p class="field-hint">(t("A directory on this machine, ssh://[user@]host/path, \
                         or s3://bucket/prefix. Leave it empty and the copy stays here, in the \
                         path the field shows — which protects against nothing that has ever \
                         happened to a disk."))</p>
                    <p class="field-hint">(t("SSH uses this machine's own keys and needs nothing \
                         stored. S3 needs its credentials in config.toml, which is deliberate: a \
                         key that can read every backup in the network must not be inside the \
                         thing being backed up."))</p>

                    <label for="backup-cadence">(t("How often"))</label>
                    <select id="backup-cadence" name="cadence">
                        @if plan.cadence == Cadence::Off {
                            <option value="" selected>(t("Never — I take them myself"))</option>
                        } @else {
                            <option value="">(t("Never — I take them myself"))</option>
                        }
                        @if plan.cadence == Cadence::Daily {
                            <option value="daily" selected>(t("Every day"))</option>
                        } @else {
                            <option value="daily">(t("Every day"))</option>
                        }
                        @if plan.cadence == Cadence::Weekly {
                            <option value="weekly" selected>(t("Every week"))</option>
                        } @else {
                            <option value="weekly">(t("Every week"))</option>
                        }
                    </select>

                    // `Cadence::Off` stores as the empty string, which is
                    // what lets `data-when="cadence"` — the island's
                    // "this control has a value" — hide the hour when
                    // nothing is scheduled. One attribute rather than a
                    // second condition and another script.
                    <div class="stack" data-when="cadence=weekly">
                        <label for="backup-weekday">(t("Day"))</label>
                        <select id="backup-weekday" name="weekday">
                            @for (index, day) in days.iter().enumerate() {
                                @if usize::from(plan.weekday) == index {
                                    <option value=(index.to_string()) selected>(day)</option>
                                } @else {
                                    <option value=(index.to_string())>(day)</option>
                                }
                            }
                        </select>
                    </div>
                    <div class="stack" data-when="cadence">
                        <label for="backup-hour">(t("At"))</label>
                        <select id="backup-hour" name="hour">
                            @for hour in 0..24u8 {
                                @if plan.hour == hour {
                                    <option value=(hour.to_string()) selected>
                                        (format!("{hour:02}:00"))
                                    </option>
                                } @else {
                                    <option value=(hour.to_string())>
                                        (format!("{hour:02}:00"))
                                    </option>
                                }
                            }
                        </select>
                        <p class="field-hint">(t("UTC, which is what the node keeps. An hour \
                             chosen in your own zone would be somebody else's afternoon, and a \
                             copy of every database is not what a busy hour needs."))</p>
                    </div>

                    <label for="backup-keep">(t("Kept for"))</label>
                    <input id="backup-keep" name="keep_days" type="text" inputmode="numeric"
                           autocomplete="off" value=(&keep_days) placeholder="7">
                    <p class="field-hint">(t("Days. Everything inside the window costs disk and \
                         everything outside it is gone — an hourly pass drops the backups that \
                         have expired, and the log no surviving backup needs. The oldest one is \
                         kept until a newer one can anchor the window, so it is never shorter \
                         than it says."))</p>

                    <label class="capability">
                        @if archiving {
                            <input type="checkbox" name="archiving" value="1" checked>
                        } @else {
                            <input type="checkbox" name="archiving" value="1">
                        }
                        <span>
                            <strong>(t("Keep the write-ahead log"))</strong>
                            <span class="tile-detail">(t("Lets a database be restored to any \
                                 minute, rather than only to the moment of a backup. It costs \
                                 disk — a segment a minute per database, compressed — and it \
                                 takes effect at each database's next deployment."))</span>
                            <span class="tile-detail">(t("It reaches back to the oldest backup \
                                 and no further: the log is replayed onto one, so without a \
                                 backup to anchor it this keeps disk nothing can use."))</span>
                        </span>
                    </label>

                    <div class="actions">
                        <button type="submit">(t("Save"))</button>
                    </div>
                </form>
            </section>
        },
    )
}

/// The bucket's credential, and where it is kept.
///
/// **Jorge asked for this**: configuring a bucket was an SSH session and
/// a text editor, which is the shape of a feature nobody sets up. What
/// does *not* change is where it lands — the config file, never the
/// database, because a key that can read every backup in the network must
/// not be inside the thing being backed up. So the card says which file,
/// and the file says the console wrote it.
///
/// **Not "AWS".** Every provider that speaks this protocol is one of
/// these: an endpoint, a region, a key and a secret. Amazon is the case
/// where the endpoint is left empty.
///
/// The secret goes in and never comes back: it renders as an empty field
/// whatever is stored, and an empty field on save means "keep the one you
/// have" — which is what makes changing a region a one-field edit rather
/// than a hunt for a secret nobody wrote down.
pub fn bucket_card<'a>(
    stored: Option<&'a crate::infra::backup::s3::Credentials>,
    config_path: &'a str,
    plan: &'a crate::infra::backup::plan::Plan,
) -> impl Renderable + 'a {
    let key = stored.map(|c| c.access_key_id.clone()).unwrap_or_default();
    let region = stored
        .map(|c| c.region.clone())
        .unwrap_or_else(|| "us-east-1".to_string());
    let endpoint = stored.and_then(|c| c.endpoint.clone()).unwrap_or_default();
    // Whether the destination is the bucket these belong to. It decides
    // one thing: forgetting them while the backups go there would leave a
    // schedule that cannot work, so that is refused rather than allowed
    // and reported afterwards.
    let in_use = plan
        .destination
        .as_deref()
        .and_then(|text| crate::infra::backup::destination::Destination::parse(text).ok())
        .is_some_and(|destination| {
            matches!(
                destination,
                crate::infra::backup::destination::Destination::S3 { .. }
            )
        });

    rsx! {
        <section class="stack">
            <p class="card-label">(t("Bucket credentials"))</p>
            <form method="post" action="/node/backups/credentials" class="card stack">
                <div class="split">
                    <span>
                        @if stored.is_some() {
                            <span class="dot dot-success"></span>(t("stored"))
                        } @else {
                            <span class="dot dot-warning"></span>(t("none on this node"))
                        }
                    </span>
                </div>
                <p class="field-hint">(t("For any storage that speaks the S3 protocol — Amazon, \
                     Backblaze, Wasabi, DigitalOcean, MinIO, Ceph. They are kept in "))
                    <span class="mono">(config_path)</span>
                    (t(", never in this node's database: a key that can read every backup in the \
                     network must not be inside the thing being backed up. A backup holds the \
                     database, the volumes and the images — it does not hold that file."))</p>

                <label for="s3-key">(t("Access key id"))</label>
                <input id="s3-key" name="access_key_id" type="text" autocomplete="off"
                       class="mono" value=(&key)>

                <label for="s3-secret">(t("Secret access key"))</label>
                // `new-password`, or the browser's password manager offers
                // the operator's own login for this console — which
                // happened once already on the sign-in form and was found
                // by somebody looking at the page, not by a test.
                <input id="s3-secret" name="secret_access_key" type="password"
                       autocomplete="new-password" class="mono" value="">
                @if stored.is_some() {
                    <p class="field-hint">(t("Stored. Leave this empty to keep it — it is never \
                         shown again, here or anywhere else."))</p>
                } @else {
                    <p class="field-hint">(t("Whatever the provider generated. It is written to \
                         the file above and never rendered back."))</p>
                }

                <label for="s3-region">(t("Region"))</label>
                <input id="s3-region" name="region" type="text" autocomplete="off" class="mono"
                       value=(&region) placeholder="us-east-1">
                <p class="field-hint">(t("Part of the signature, so a wrong one is a refused \
                     request rather than a slow one. Providers that have no regions still want \
                     the one they document — often us-east-1."))</p>

                <label for="s3-endpoint">(t("Endpoint"))</label>
                <input id="s3-endpoint" name="endpoint" type="text" autocomplete="off"
                       class="mono" value=(&endpoint)
                       placeholder="https://s3.us-west-004.backblazeb2.com">
                <p class="field-hint">(t("Empty for Amazon's own S3. For anything else, the URL \
                     the provider gives you — and with one set, buckets are addressed as a path \
                     rather than a subdomain, which is what MinIO and most of the others \
                     want."))</p>

                <p class="field-hint">(t("Saving asks the bucket to list what it holds, when the \
                     destination above is one — the same call a backup starts with, so a key \
                     that saves is a key that works."))</p>

                <div class="actions">
                    <button type="submit">(t("Save"))</button>
                </div>
            </form>

            // Its own form: a second button inside the one above would
            // submit the fields with it, and forgetting a credential must
            // not depend on what is typed beside it.
            @if stored.is_some() {
                <form method="post" action="/node/backups/credentials/forget" class="card stack">
                    <p class="field-hint">(t("Removing them takes the section out of the file. \
                         Nothing else about the plan changes — and backups to a bucket stop \
                         working, which is why this is refused while the destination is one."))</p>
                    <div class="actions">
                        @if in_use {
                            <button class="btn btn-secondary" type="submit" disabled>
                                (t("Forget them"))
                            </button>
                        } @else {
                            <button class="btn btn-secondary" type="submit">(t("Forget them"))</button>
                        }
                    </div>
                </form>
            }
        </section>
    }
}

/// How the last one went, and the button that takes one now.
///
/// A card of its own rather than a line on the form: what it says is not
/// a setting, and the two questions it answers — "is there a copy of this
/// node anywhere" and "did the last attempt work" — are the ones somebody
/// opens this tab to ask.
pub fn backup_state_card<'a>(
    plan: &'a crate::infra::backup::plan::Plan,
    attempt: &'a crate::infra::backup::plan::Attempt,
    taken: &'a [(std::path::PathBuf, crate::infra::backup::Manifest)],
    self_id: &'a str,
    running: bool,
) -> impl Renderable + 'a {
    use crate::infra::backup::plan::Cadence;

    let next = crate::infra::backup::plan::next_slot(plan, crate::infra::clock::now_ms());
    // Ten, and the count says what was left out. A node kept for a year
    // with a daily backup would otherwise render a table nobody scrolls.
    let showing: Vec<&(std::path::PathBuf, crate::infra::backup::Manifest)> =
        taken.iter().take(10).collect();
    let remote = plan
        .destination
        .as_deref()
        .and_then(|text| crate::infra::backup::destination::Destination::parse(text).ok())
        .is_some_and(|destination| destination.is_remote());

    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("The last one"))</p>
                // Not a form field: a button that submits nothing but the
                // request itself. Its own form because a POST inside the
                // settings form would save the plan as a side effect of
                // asking for a backup.
                <form method="post" action="/node/backups/now">
                    @if running {
                        <button class="btn btn-secondary" type="submit" disabled>
                            (t("Backing up…"))
                        </button>
                    } @else {
                        <button class="btn btn-secondary" type="submit">(t("Back up now"))</button>
                    }
                </form>
            </div>

            <dl class="kv">
                <dt>(t("Last attempt"))</dt>
                <dd>
                    @if running {
                        <span class="dot dot-info"></span>(t("running now"))
                    } @else if let Some(at) = attempt.at {
                        (layout::when(at))
                    } @else {
                        (t("never from here"))
                    }
                </dd>
                @if let Some(at) = attempt.ok_at {
                    <dt>(t("Last good one"))</dt>
                    // Absolute as well as relative: this is the moment a
                    // restore would land on, and "3 days ago" cannot be
                    // typed into anything.
                    <dd>(layout::when(at))(" · ")<span class="mono">(layout::exactly(at))</span></dd>
                }
                @if let Some(note) = &attempt.note {
                    <dt>(t("It held"))</dt>
                    <dd>(note)</dd>
                }
                <dt>(t("Next"))</dt>
                <dd>
                    @if plan.cadence == Cadence::Off {
                        (t("nothing is scheduled"))
                    } @else if let Some(at) = next {
                        <span class="mono">(layout::exactly(at))</span>
                    } @else {
                        (t("nothing is scheduled"))
                    }
                </dd>
            </dl>

            // The reason the last one failed, where somebody looking for
            // it will be. It is whatever refused — rsync's words, or an
            // authority's — so it renders as it arrived.
            @if let Some(reason) = &attempt.error {
                <p class="form-error">
                    <strong>(t("The last attempt did not work: "))</strong>(reason)
                </p>
                <p class="field-hint">(t("A slot gets one attempt: a copy of every database \
                     retried every few minutes is a machine that stops answering. Fix what it \
                     names and press Back up now, or wait for the next one."))</p>
            }

            @if showing.is_empty() {
                <p class="note">(t("Nothing on this machine. A backup taken here and sent \
                     somewhere else leaves nothing behind, which is the point — this list is \
                     what is still on this disk."))</p>
            } @else {
                <table>
                    <thead>
                        <tr>
                            <th>(t("Taken"))</th>
                            <th>(t("Holds"))</th>
                            <th>(t("Of"))</th>
                        </tr>
                    </thead>
                    <tbody>
                        @for (path, manifest) in &showing {
                            (backup_row(path, manifest, self_id))
                        }
                    </tbody>
                </table>
                @if taken.len() > showing.len() {
                    <p class="note">(t("Showing the newest "))(showing.len().to_string())
                        (t(" of "))(taken.len().to_string())(t(" on this disk."))</p>
                }
            }
            @if remote {
                <p class="note">(t("The copies are sent away and the local staging directory is \
                     removed when the transfer succeeds, so this list is not what the \
                     destination holds."))</p>
            }
        </section>
    }
}

/// One backup on the disk.
pub fn backup_row<'a>(
    path: &'a std::path::Path,
    manifest: &'a crate::infra::backup::Manifest,
    self_id: &'a str,
) -> impl Renderable + 'a {
    let crash = manifest
        .volumes
        .iter()
        .filter(|copied| copied.how == crate::infra::backup::How::CrashConsistent)
        .count();
    // Whose it is. A shared root holds several nodes' backups — that is
    // what makes one bucket the answer for a network — so a row about
    // another machine has to say so rather than read as this one's.
    let whose = match manifest.node_id.as_deref() {
        Some(id) if id == self_id => t("this node").to_string(),
        Some(_) => manifest
            .node_name
            .clone()
            .or_else(|| manifest.node_id.clone())
            .unwrap_or_else(|| t("another node").to_string()),
        None => t("a node with no id").to_string(),
    };

    rsx! {
        <tr>
            <td>
                <span class="mono">(layout::exactly(manifest.taken_at))</span>
                // The directory, because moving it off this machine is
                // the one thing an operator will do with it and `scp`
                // wants the path.
                //
                // **A `<p>`, not a span.** `.tile-detail` carries no
                // margin of its own, so a span sits flush against what
                // is beside it: on the node this rendered as
                // `02:43 UTC/var/lib/…`, one word, and Jorge saw it
                // before any test did. The class is the same either way
                // — what makes it a second line is the element.
                <p class="tile-detail mono">(path.display().to_string())</p>
            </td>
            <td>
                (manifest.volumes.len().to_string())(t(" volume(s)"))
                @if crash > 0 {
                    // Named, not folded in. A file copy of a running
                    // process may restore into a process that will not
                    // start, and which of the two this is decides whether
                    // the backup is worth anything.
                    <p class="tile-detail">(crash.to_string())(t(" copied while running"))</p>
                }
            </td>
            <td>(whose)</td>
        </tr>
    }
}
