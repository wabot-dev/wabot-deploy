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

use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::errors::PlatformResult;
use crate::features::node::concern_response::{ConcernResponse, Weight};
use crate::infra::config::Config;
use crate::infra::deploy::Deployer;

/// Read every source once.
///
/// Ordered worst first, and within a weight in the order the sources are
/// asked — which puts what is failing above what is merely untidy.
///
/// One `Vec`, not a stream: this runs when a page is drawn, and every
/// source is a query the node already makes elsewhere.
pub async fn run(
    database: &Arc<SqliteDatabase>,
    config: &Config,
    deployer: &Arc<Deployer>,
    account: &Account,
) -> PlatformResult<Vec<ConcernResponse>> {
    if !node_user::manages_the_machine(account) {
        return Ok(Vec::new());
    }
    let state = &Sources {
        database,
        config,
        deployer,
    };
    let mut concerns = Vec::new();

    copies_that_failed(state, &mut concerns).await?;
    copies_that_stopped_answering(state, &mut concerns).await;
    a_database_with_every_copy_on_one_machine(state, &mut concerns).await;
    certificates_that_would_not_issue(state, &mut concerns).await?;
    errands_that_were_refused(state, &mut concerns).await;
    what_nothing_claims(state, &mut concerns).await;
    a_backup_that_did_not_work(state, &mut concerns).await;
    nodes_this_one_cannot_reach(state, &mut concerns).await;

    concerns.sort_by_key(|concern| std::cmp::Reverse(concern.weight));
    Ok(concerns)
}

/// A copy the node tried to start and could not.
///
/// The most direct of these: somebody deployed something and it is not
/// running. The reason is the container's own last words, which
/// `deploy::logs` keeps for exactly this.
async fn copies_that_failed(
    state: &Sources<'_>,
    concerns: &mut Vec<ConcernResponse>,
) -> PlatformResult<()> {
    let services = crate::infra::repositories::services::all(state.database, None).await?;
    let projects = crate::infra::repositories::projects::all(state.database).await?;

    for replica in crate::infra::repositories::replicas::here(state.database).await? {
        let Some(reason) = &replica.last_error else {
            continue;
        };
        let Some(service) = services.iter().find(|s| s.id == replica.service_id) else {
            continue;
        };
        let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
            continue;
        };
        concerns.push(ConcernResponse {
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
async fn copies_that_stopped_answering(state: &Sources<'_>, concerns: &mut Vec<ConcernResponse>) {
    let Ok(services) = crate::infra::repositories::services::all(state.database, None).await else {
        return;
    };
    let Ok(projects) = crate::infra::repositories::projects::all(state.database).await else {
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
        concerns.push(ConcernResponse {
            weight: Weight::Wrong,
            what: "Copies out of the rotation".into(),
            which: format!("{} · {}", service.slug, silent.len()),
            why: String::new(),
            go: format!("/projects/{}/services/{}", project.slug, service.slug),
        });
    }
}

/// A database whose copies all sit on one machine.
///
/// **A standby beside its own primary is capacity, not redundancy.** It
/// keeps its own CPU allocation and its own engine process, so it goes
/// on serving reads and is worth having — that is why the planner places
/// it rather than refusing. What it does not do is survive the machine,
/// and a page that shows two copies without saying so is a page somebody
/// plans around.
///
/// **Found on the nodes**, 2026-08-25: draining the only other node
/// moved the only remote standby home, wrote the reason on the row, and
/// nothing anywhere treated it as worth reading. `Notice` and not
/// `Wrong` — nothing failed, and this may be exactly what the operator
/// meant on a one-machine network.
async fn a_database_with_every_copy_on_one_machine(
    state: &Sources<'_>,
    concerns: &mut Vec<ConcernResponse>,
) {
    let (Ok(services), Ok(projects)) = (
        crate::infra::repositories::services::all(state.database, None).await,
        crate::infra::repositories::projects::all(state.database).await,
    ) else {
        return;
    };

    for service in services.iter().filter(|s| s.kind.is_managed()) {
        let Ok(copies) =
            crate::infra::repositories::replicas::of_service(state.database, &service.id).await
        else {
            continue;
        };
        let live: Vec<_> = copies.iter().filter(|copy| !copy.evicted()).collect();
        // One copy is not a redundancy claim, so it is not this
        // concern — a database with no standby is a decision, and a
        // different sentence if it is ever worth one.
        if live.len() < 2 {
            continue;
        }
        let machines: std::collections::BTreeSet<Option<&str>> =
            live.iter().map(|copy| copy.node_id.as_deref()).collect();
        if machines.len() > 1 {
            continue;
        }
        let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
            continue;
        };
        concerns.push(ConcernResponse {
            weight: Weight::Notice,
            what: "Every copy of a database on one machine".into(),
            which: format!("{} · {}", service.slug, live.len()),
            why: String::new(),
            go: format!(
                "/projects/{}/services/{}/settings/replicas",
                project.slug, service.slug
            ),
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
    state: &Sources<'_>,
    concerns: &mut Vec<ConcernResponse>,
) -> PlatformResult<()> {
    if let Some(reason) = crate::infra::node::settings::acme_error(state.database).await {
        concerns.push(ConcernResponse {
            weight: Weight::Wrong,
            what: "A certificate would not issue".into(),
            which: crate::infra::node::settings::domain(state.database, state.config)
                .await
                .unwrap_or_else(|| "this node".into()),
            why: first_line(&reason),
            go: "/network".into(),
        });
    }

    for port in crate::infra::repositories::ports::all(state.database).await? {
        let Some(hostname) = &port.hostname else {
            continue;
        };
        let policy =
            crate::infra::edge::policy::for_name(state.database, state.config, hostname).await;
        let Some(reason) = &policy.last_error else {
            continue;
        };
        concerns.push(ConcernResponse {
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
async fn errands_that_were_refused(state: &Sources<'_>, concerns: &mut Vec<ConcernResponse>) {
    let Ok(records) = crate::infra::network::errand::all(state.database).await else {
        return;
    };
    let nodes = crate::infra::network::all(state.database)
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
        concerns.push(ConcernResponse {
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
async fn what_nothing_claims(state: &Sources<'_>, concerns: &mut Vec<ConcernResponse>) {
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
    let Some(claims) = crate::infra::deploy::Deployer::claimed(state.database).await else {
        return;
    };
    let live = crate::infra::deploy::Claim::containers(&claims);
    let leftovers = crate::infra::deploy::Deployer::leftovers(&state.config.node.data_dir, &live);
    if leftovers.is_empty() {
        return;
    }
    concerns.push(ConcernResponse {
        weight: Weight::Notice,
        what: "Storage nothing claims".into(),
        which: format!("{}", leftovers.len()),
        why: String::new(),
        // This node's own page, which is where the disk card is. `/nodes`
        // is the list of machines and says nothing about storage, so the
        // link answered "see" with a page that had nothing to see —
        // reported alongside the false count, and a separate fault from
        // it.
        go: match crate::infra::network::me(state.database).await {
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
async fn a_backup_that_did_not_work(state: &Sources<'_>, concerns: &mut Vec<ConcernResponse>) {
    let Some(reason) = crate::infra::backup::plan::last(state.database).await.error else {
        return;
    };
    let plan = crate::infra::backup::plan::plan(state.database).await;
    concerns.push(ConcernResponse {
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
        go: "/node/backup".into(),
    });
}

/// Nodes this one is trying to reach and cannot, and nodes it has no way
/// to reach at all.
///
/// **Two different things, and only one is a fault.** A peer this node
/// configured an endpoint for and has never been answered by is broken: we
/// believe we can reach it. A pair of private nodes with nowhere to dial is
/// *geometry* — neither has an address the other can use, and on the
/// internet that has no answer — so it is a notice, and it is named because
/// unnamed it looks exactly like the first one.
///
/// One netlink read, like the disk walk beside it, and silence when there
/// is no interface: a node whose kernel refuses WireGuard has a console to
/// find out why, and this must not be what breaks it.
async fn nodes_this_one_cannot_reach(state: &Sources<'_>, concerns: &mut Vec<ConcernResponse>) {
    let (Ok(interface), Ok(Some(me))) = (
        crate::infra::network::tunnel::observed(),
        crate::infra::network::me(state.database).await,
    ) else {
        return;
    };
    let Ok(nodes) = crate::infra::network::all(state.database).await else {
        return;
    };

    for paired in crate::infra::network::tunnel::pair(&me, &nodes, &interface) {
        use crate::infra::network::tunnel::Path;
        let go = format!("/network/{}", paired.node_id);
        // Written out rather than through a variable: the guard below
        // checks that each of these words appears in this file as a
        // literal, which is what stops a wording drifting away from its
        // Spanish.
        match paired.path {
            Path::Dialling { at } => concerns.push(ConcernResponse {
                weight: Weight::Wrong,
                what: "A node this one cannot reach".into(),
                which: paired.name,
                why: format!("configured at {at}, and it has never answered"),
                go,
            }),
            Path::Nowhere => concerns.push(ConcernResponse {
                weight: Weight::Notice,
                what: "No way to reach a node".into(),
                which: paired.name,
                why: "both are behind a router and not on one network".into(),
                go,
            }),
            _ => continue,
        }
    }

    // And an address two nodes claim, which the census refuses to write
    // rather than letting the unique index fail into a log. A peer that
    // can never work.
    if let Some(said) = crate::infra::network::roster::conflicts(state.database).await {
        concerns.push(ConcernResponse {
            weight: Weight::Wrong,
            what: "Two nodes claim one address".into(),
            which: String::new(),
            why: said,
            go: "/network".into(),
        });
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

struct Sources<'a> {
    database: &'a Arc<SqliteDatabase>,
    config: &'a Config,
    deployer: &'a Arc<Deployer>,
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
            "A node this one cannot reach",
            "No way to reach a node",
            "Two nodes claim one address",
        ] {
            assert!(
                crate::shell::texts::lookup(what, crate::features::node::texts::TABLE).is_some(),
                "no Spanish for the concern {what:?}"
            );
            // And it is one this module actually produces. A guard
            // listing words nobody writes passes for ever while the real
            // ones drift past it.
            assert!(
                include_str!("node_user_see_what_needs_attention.rs")
                    .contains(&format!("{what:?}.into()")),
                "{what:?} is guarded here and produced nowhere"
            );
        }
    }

    /// Worst first. An operator reading top to bottom must meet what is
    /// broken before what is untidy, or the ordering is decoration.
    #[test]
    fn what_is_failing_sorts_above_what_is_untidy() {
        let mut concerns = [
            ConcernResponse {
                weight: Weight::Notice,
                what: "Storage nothing claims".into(),
                which: "3".into(),
                why: String::new(),
                go: "/network".into(),
            },
            ConcernResponse {
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

    /// The backup concern links to the Backup tab by its path, because a
    /// use case may not import the page the tab belongs to. This is what
    /// keeps the two spellings one.
    #[test]
    fn the_backup_concern_goes_to_the_backup_tab() {
        let source = include_str!("node_user_see_what_needs_attention.rs");
        let linked = format!(
            "go: {:?}.into()",
            crate::features::node::node_user_node_settings_page::NodeTab::Backup.path()
        );
        assert!(
            source.contains(&linked),
            "the backup concern no longer links to {linked}"
        );
    }
}
