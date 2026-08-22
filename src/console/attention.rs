//! Everything that needs somebody, in one place.
//!
//! ## Aggregation, not instrumentation
//!
//! **The node already knows all of this.** A copy that failed to start
//! is on `replica.last_error`; a name whose certificate would not issue
//! is on its policy row; an errand that was refused is on the errand
//! row; an upstream nobody answers on is in the edge's health map; a
//! directory nothing claims is on the disk. Every one of them was
//! already true and already stored, and an operator had to visit five
//! pages to assemble the picture — which means the picture only got
//! assembled by somebody who already suspected something.
//!
//! Nothing here measures anything new. It reads what six other things
//! wrote and puts it where a person lands.
//!
//! ## A concern names what to do, or it does not belong here
//!
//! Every entry carries a link to the page where the thing can be acted
//! on. A list of complaints with nowhere to go is a list people learn to
//! scroll past — and this console has the opposite habit written down
//! everywhere else: *errors are values somebody can act on*.
//!
//! ## What is deliberately not a concern
//!
//! A service somebody stopped. A replica evicted by the node holding it,
//! which is that node's decision reported faithfully. An update
//! available. None of these is wrong; a page that cried about them would
//! be a page whose red dot means "read me sometimes".

use hypertext::prelude::*;
use hypertext::Renderable;

use super::language::t;
use crate::platform::PlatformResult;

/// How much somebody should care.
///
/// Two levels, not five. The question this page answers is "do I have to
/// do something now or not", and a scale with a middle is a scale where
/// everything ends up in the middle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Weight {
    /// Worth knowing, nothing is failing because of it.
    Notice,
    /// Something is not working, and it will not fix itself.
    Wrong,
}

impl Weight {
    pub fn badge(&self) -> &'static str {
        match self {
            Self::Wrong => "badge badge-danger",
            Self::Notice => "badge badge-warning",
        }
    }

    pub fn dot(&self) -> &'static str {
        match self {
            Self::Wrong => "dot dot-danger",
            Self::Notice => "dot dot-warning",
        }
    }
}

/// One thing that needs somebody.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Concern {
    pub weight: Weight,
    /// What it is, in the fewest words that are still true. Translated
    /// through `language::word`, because it is chosen here and rendered
    /// elsewhere — the same door every badge word goes through.
    pub what: String,
    /// Which thing: a service's name, a hostname, a node's name, a path.
    /// Never translated — it is somebody's own string or the machine's.
    pub which: String,
    /// The reason, as whatever wrote it put it. Also never translated:
    /// it is containerd's words, or an authority's, or a filesystem's.
    pub why: String,
    /// Where to go and do something about it.
    pub go: String,
}

/// Read every source once.
///
/// Ordered worst first, and within a weight in the order the sources are
/// asked — which puts what is failing above what is merely untidy.
///
/// One `Vec`, not a stream: this runs when a page is drawn, and every
/// source is a query the node already makes elsewhere.
pub async fn gather(state: &super::ConsoleState) -> PlatformResult<Vec<Concern>> {
    let mut concerns = Vec::new();

    copies_that_failed(state, &mut concerns).await?;
    copies_that_stopped_answering(state, &mut concerns).await;
    certificates_that_would_not_issue(state, &mut concerns).await?;
    errands_that_were_refused(state, &mut concerns).await;
    what_nothing_claims(state, &mut concerns).await;
    a_backup_that_did_not_work(state, &mut concerns).await;

    concerns.sort_by_key(|concern| std::cmp::Reverse(concern.weight));
    Ok(concerns)
}

/// A copy the node tried to start and could not.
///
/// The most direct of these: somebody deployed something and it is not
/// running. The reason is the container's own last words, which
/// `deploy::logs` keeps for exactly this.
async fn copies_that_failed(
    state: &super::ConsoleState,
    concerns: &mut Vec<Concern>,
) -> PlatformResult<()> {
    let services = crate::platform::services::all(&state.database, None).await?;
    let projects = crate::platform::projects::all(&state.database).await?;

    for replica in crate::platform::replicas::here(&state.database).await? {
        let Some(reason) = &replica.last_error else {
            continue;
        };
        let Some(service) = services.iter().find(|s| s.id == replica.service_id) else {
            continue;
        };
        let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
            continue;
        };
        concerns.push(Concern {
            weight: Weight::Wrong,
            what: "A copy will not start".into(),
            which: format!("{} #{}", service.slug, replica.slot),
            why: first_line(reason),
            go: format!("/projects/{}/services/{}", project.slug, service.slug),
        });
    }
    Ok(())
}

/// A copy that is running and answering nothing.
///
/// Distinct from the one above and worth its own line: nothing failed,
/// nothing was reported, and the edge quietly stopped sending it
/// traffic. Without this the only sign is a service that is slower than
/// it should be.
async fn copies_that_stopped_answering(state: &super::ConsoleState, concerns: &mut Vec<Concern>) {
    let Ok(services) = crate::platform::services::all(&state.database, None).await else {
        return;
    };
    let Ok(projects) = crate::platform::projects::all(&state.database).await else {
        return;
    };

    for service in &services {
        let silent = state.deployer.not_answering(service).await;
        if silent.is_empty() {
            continue;
        }
        let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
            continue;
        };
        concerns.push(Concern {
            weight: Weight::Wrong,
            what: "Copies out of the rotation".into(),
            which: format!("{} · {}", service.slug, silent.len()),
            why: String::new(),
            go: format!("/projects/{}/services/{}", project.slug, service.slug),
        });
    }
}

/// A name whose certificate would not issue.
///
/// Both shapes: the node's own last ACME failure, and a per-name one.
/// This is the concern most worth surfacing early, because the thing it
/// leads to — an authority that locks an account after five failed
/// validations in an hour — is not undone by fixing the cause later.
async fn certificates_that_would_not_issue(
    state: &super::ConsoleState,
    concerns: &mut Vec<Concern>,
) -> PlatformResult<()> {
    if let Some(reason) = crate::node::settings::acme_error(&state.database).await {
        concerns.push(Concern {
            weight: Weight::Wrong,
            what: "A certificate would not issue".into(),
            which: crate::node::settings::domain(&state.database, &state.config)
                .await
                .unwrap_or_else(|| "this node".into()),
            why: first_line(&reason),
            go: "/network".into(),
        });
    }

    for port in crate::platform::ports::all(&state.database).await? {
        let Some(hostname) = &port.hostname else {
            continue;
        };
        let policy = crate::edge::policy::for_name(&state.database, &state.config, hostname).await;
        let Some(reason) = &policy.last_error else {
            continue;
        };
        concerns.push(Concern {
            weight: Weight::Wrong,
            what: "A certificate would not issue".into(),
            which: hostname.clone(),
            why: first_line(reason),
            go: "/network".into(),
        });
    }
    Ok(())
}

/// An instruction another node refused, or could not carry out.
///
/// A failure is an answer — that is why it is stored rather than
/// retried — but an answer nobody reads is the same as no answer. The
/// case that produced this: a node asked to serve a name it had never
/// agreed to serve, refused correctly, and said so to a table nothing
/// displayed.
async fn errands_that_were_refused(state: &super::ConsoleState, concerns: &mut Vec<Concern>) {
    let Ok(records) = crate::network::errand::all(&state.database).await else {
        return;
    };
    let nodes = crate::network::all(&state.database)
        .await
        .unwrap_or_default();

    for record in records.iter().filter(|record| record.failed()) {
        let Some(reason) = &record.error else {
            continue;
        };
        let name = nodes
            .iter()
            .find(|node| node.id == record.node_id)
            .map(|node| node.name.clone())
            .unwrap_or_else(|| record.node_id.clone());
        concerns.push(Concern {
            weight: Weight::Notice,
            what: "An instruction was refused".into(),
            which: name,
            why: first_line(reason),
            go: "/network".into(),
        });
    }
}

/// Storage nothing claims.
///
/// A notice, never a fault: `doctor` has reported these since volumes
/// existed and the rule has always been that a directory this node
/// cannot explain is one somebody can still recover from. What it costs
/// is disk, slowly, which is why it belongs on a page rather than only
/// in a command somebody runs when they already suspect something.
async fn what_nothing_claims(state: &super::ConsoleState, concerns: &mut Vec<Concern>) {
    // Deliberately not the disk walk: this is a `read_dir` per kind
    // against a list of live ids, and it runs when a page is drawn.
    //
    // **The list has to be the real one.** It was `Vec::new()`, so every
    // directory on the disk compared as unclaimed and the card fired on
    // any node running anything — Jorge's said three, followed the link,
    // and found nothing wrong, because nothing was. A notice that cries
    // wolf is worse than no notice: this card's whole value is being
    // absent when there is nothing, and it cannot be absent if it counts
    // healthy services.
    //
    // `Deployer::claimed` is the one derivation of what this node claims,
    // shared with `doctor` and `backup` — and `None` from it means the
    // rows could not be read, which is not grounds for calling anything
    // rubbish. Silence then, rather than a count of everything.
    let Some(claims) = crate::deploy::Deployer::claimed(&state.database).await else {
        return;
    };
    let live = crate::deploy::Claim::containers(&claims);
    let leftovers = crate::deploy::Deployer::leftovers(&state.config.node.data_dir, &live);
    if leftovers.is_empty() {
        return;
    }
    concerns.push(Concern {
        weight: Weight::Notice,
        what: "Storage nothing claims".into(),
        which: format!("{}", leftovers.len()),
        why: String::new(),
        // This node's own page, which is where the disk card is. `/nodes`
        // is the list of machines and says nothing about storage, so the
        // link answered "see" with a page that had nothing to see —
        // reported alongside the false count, and a separate fault from
        // it.
        go: match crate::network::me(&state.database).await {
            Ok(Some(me)) => format!("/network/{}", me.id),
            _ => "/network".into(),
        },
    });
}

/// A backup this node tried to take and could not.
///
/// **Wrong, not a notice.** What it means is that there is no copy of
/// this node from the moment somebody believes there is — and the entire
/// value of a schedule is not having to check. A destination that has
/// been refusing for a week is invisible in a journal nobody reads, which
/// is why `node::backups` stores the reason rather than only logging it.
///
/// Nothing here complains about a node that has *no* schedule. That is a
/// decision, not a failure: an operator whose cron already runs
/// `wabot-deploy backup` has made it, and a card they cannot silence is a
/// card they learn to ignore.
async fn a_backup_that_did_not_work(state: &super::ConsoleState, concerns: &mut Vec<Concern>) {
    let Some(reason) = crate::node::backups::last(&state.database).await.error else {
        return;
    };
    let plan = crate::node::backups::plan(&state.database).await;
    concerns.push(Concern {
        weight: Weight::Wrong,
        what: "A backup did not work".into(),
        // Where it was going, which is the part that is usually wrong.
        // Somebody else's string or this machine's path — never
        // translated, like every other `which`.
        which: plan.destination.unwrap_or_else(|| {
            state
                .config
                .node
                .data_dir
                .join("backups")
                .display()
                .to_string()
        }),
        why: first_line(&reason),
        // The tab, by name rather than by string: a slug that moved would
        // otherwise leave a concern linking to a redirect.
        go: super::nodes::NodeTab::Backup.path(),
    });
}

/// The card, or nothing at all.
///
/// **Nothing when there is nothing**, which is the property that makes
/// the rest of it worth anything: a panel that is always on the page
/// becomes part of the wallpaper, and the day it has something to say
/// it says it in the same place it says "all well". An operator should
/// be able to tell at a glance, from whether the card is there.
///
/// Not a stream. These are read when the page is drawn, and a page an
/// operator is looking at is one they can reload — where a badge that
/// updates itself would mean a list that reorders under a cursor
/// half-way to clicking something.
pub fn card(concerns: &[Concern]) -> impl Renderable + '_ {
    rsx! {
        @if !concerns.is_empty() {
            <section class="stack">
                <p class="card-label">(t("Needs you"))</p>
                <div class="card">
                    <table>
                        <tbody>
                            @for concern in concerns {
                                <tr>
                                    <td>
                                        <span class=(concern.weight.badge())>
                                            <span class=(concern.weight.dot())></span>
                                            (super::language::word(&concern.what))
                                        </span>
                                    </td>
                                    // Somebody's own string, or the
                                    // machine's: never translated, like
                                    // every hostname and slug here.
                                    <td class="mono">(&concern.which)</td>
                                    <td class="tile-detail">(&concern.why)</td>
                                    <td>
                                        <a class="btn btn-ghost btn-sm"
                                           href=(&concern.go)>(t("Look"))</a>
                                    </td>
                                </tr>
                            }
                        </tbody>
                    </table>
                </div>
            </section>
        }
    }
}

/// The first line of a reason, for a list.
///
/// A container's last words can be a hundred lines of Postgres telling
/// you what it thought about its configuration, and a list where one
/// entry is a screen is a list nobody can read. The whole of it is on
/// the page the entry links to.
fn first_line(reason: &str) -> String {
    reason.lines().next().unwrap_or_default().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `what` this module can produce has Spanish behind it.
    ///
    /// They reach the render through `language::word` as values, so the
    /// source scan in `es.rs` cannot see them — the same door the badge
    /// words go through, and the same guard they needed. Written the
    /// moment this module was, because the last two times the words were
    /// right by luck and the one added afterwards was not.
    ///
    /// Named by hand, which is the cost of that door: a `what` added
    /// below and not here is a Spanish page with an English line on it.
    #[test]
    fn every_concern_is_a_sentence_somebody_reads() {
        for what in [
            "A copy will not start",
            "Copies out of the rotation",
            "A certificate would not issue",
            "An instruction was refused",
            "Storage nothing claims",
            "A backup did not work",
        ] {
            assert!(
                crate::console::es::lookup(what).is_some(),
                "no Spanish for the concern {what:?}"
            );
            // And it is one this module actually produces. A guard
            // listing words nobody writes passes for ever while the real
            // ones drift past it.
            assert!(
                include_str!("attention.rs").contains(&format!("{what:?}.into()")),
                "{what:?} is guarded here and produced nowhere"
            );
        }
    }

    /// Worst first. An operator reading top to bottom must meet what is
    /// broken before what is untidy, or the ordering is decoration.
    #[test]
    fn what_is_failing_sorts_above_what_is_untidy() {
        let mut concerns = [
            Concern {
                weight: Weight::Notice,
                what: "Storage nothing claims".into(),
                which: "3".into(),
                why: String::new(),
                go: "/network".into(),
            },
            Concern {
                weight: Weight::Wrong,
                what: "A copy will not start".into(),
                which: "api #1".into(),
                why: "it exited 1".into(),
                go: "/projects/demo/services/api".into(),
            },
        ];
        concerns.sort_by_key(|concern| std::cmp::Reverse(concern.weight));
        assert_eq!(concerns[0].weight, Weight::Wrong);
    }

    /// A reason is one line here and the whole story on the page it
    /// links to. Postgres explaining its configuration is a hundred
    /// lines, and a list where one entry fills the screen is a list
    /// nobody reads.
    #[test]
    fn a_reason_is_one_line_in_a_list() {
        assert_eq!(
            first_line("FATAL: recovery aborted\nDETAIL: max_connections = 10\nHINT: restart"),
            "FATAL: recovery aborted"
        );
        assert_eq!(first_line("  padded  "), "padded");
        assert_eq!(first_line(""), "");
    }
}
