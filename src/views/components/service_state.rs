//! What a service is doing, and the buttons that change it.
//!
//! Status is a coloured dot plus a word — never a colour alone, which
//! says nothing to somebody who cannot tell them apart.

use hypertext::prelude::*;

use crate::views::texts::language::t;

/// Rendered from the same [`StateCell`] the stream sends, so the first
/// paint and every update after it come from one place. They used to be
/// two functions that had to be kept in step, which is the kind of pair
/// that drifts.
pub fn state_badge(cell: &StateCell) -> impl Renderable + '_ {
    rsx! {
        <span class=(cell.badge)>
            <span class=(cell.dot)></span>(crate::views::texts::language::word(&cell.word))
        </span>
    }
}

/// One control, disabled or not.
///
/// Written twice rather than `disabled=(busy)`, for the same reason:
/// `disabled="false"` disables. There is no way to spell "no attribute"
/// in an `rsx!` attribute value, so the branch is on the element.
/// The play and the stop, for whichever page is showing a service.
///
/// One definition, because the project's list and the service's own
/// page must not disagree about which control applies — and they had
/// no way not to, since the detail page simply had neither. A service
/// you can see the state of and cannot start is a page that shows a
/// problem and withholds the fix.
///
/// Both actions are rendered and one is hidden, so the stream can swap
/// them by toggling: an island may hide what does not apply, not build
/// what does. The check that counts is on the POST; this is so the page
/// does not offer what it will refuse.
pub fn deploy_controls<'a>(
    project_slug: &'a str,
    service_slug: &'a str,
    cell: &'a StateCell,
    // Where to go afterwards. The form carries it because only the form
    // knows which page it is on — the same two controls live on the
    // list and on the service's own page.
    from: &'a str,
) -> impl Renderable + 'a {
    rsx! {
        <form method="post" data-action="deploy"
              class=(crate::views::components::action_button::shown(cell.action == "deploy"))
              action=(format!("/projects/{project_slug}/services/{service_slug}/deploy"))>
            <input type="hidden" name="from" value=(from)>
            (crate::views::components::action_button::action_button("Deploy", crate::views::components::action_button::PLAY, cell.busy))
        </form>
        <form method="post" data-action="stop"
              class=(crate::views::components::action_button::shown(cell.action == "stop"))
              action=(format!("/projects/{project_slug}/services/{service_slug}/stop"))>
            <input type="hidden" name="from" value=(from)>
            (crate::views::components::action_button::action_button("Stop", crate::views::components::action_button::STOP, cell.busy))
        </form>
    }
}

/// One service's state, as the three things the badge shows.
///
/// Formatted here so the first paint and every update after it come
/// from one place. The stream only assigns text and classes that the
/// server already rendered — see the island rules in CLAUDE.md.
#[derive(serde::Serialize)]
pub struct StateCell {
    pub word: String,
    pub badge: &'static str,
    pub dot: &'static str,
    /// Which control applies: `deploy` or `stop`. Always one of the
    /// two — a control that vanishes takes the column's width with it
    /// and leaves nothing to press or to read.
    pub action: &'static str,
    /// Whether it is pressable. A deployment in flight shows `stop`
    /// disabled: that is where it is heading, and saying so is more
    /// use than an empty cell.
    pub busy: bool,
    /// Where it runs, in the reader's language.
    ///
    /// Written by [`LiveState::translate`], which is the only thing that
    /// reads it: the page composes the same sentence from
    /// [`StateCell::whereabouts`] inside its own render scope. Two
    /// callers of one function rather than one value passed through two
    /// places that would each have to remember to translate it.
    pub address: String,
    /// The parts of that sentence, before it is one.
    ///
    /// A count and a language are decided in different places — the
    /// count in a handler, which awaits, and the language around a
    /// render, which cannot. So the handler carries the count and the
    /// render says it.
    #[serde(skip)]
    pub whereabouts: Whereabouts,
}

/// How many copies of a service there are, and how many of them are on
/// other machines.
///
/// A count rather than an address, everywhere. The single-copy case used
/// to name the container's address, which put a `10.42.x.x` under a
/// column headed "Address" on one row and a count on the next — and that
/// address is reachable from inside the project only, where the
/// service's own name resolves and is the thing to use. Where each copy
/// is and what it is doing belongs to the replicas table, one row each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Whereabouts {
    pub total: usize,
    pub away: usize,
}

impl Whereabouts {
    /// The sentence, in the language of the render in progress.
    ///
    /// The number is a number in both languages and the nouns are not,
    /// so the nouns go through `t` and the sentence is composed around
    /// them. It was composed whole in the handler and reached neither
    /// door — `t` takes a literal and this is not one, and the stream's
    /// `translate` looks a whole value up in a table, which no sentence
    /// with a count in it can ever be found in. So a Spanish console
    /// read "3 replicas · 2 elsewhere".
    pub fn say(&self) -> String {
        match (self.total, self.away) {
            (0, _) => t("nowhere").to_string(),
            // A copy that is not here is worth saying even when it is
            // the only one: "running" on a page served by this node
            // would otherwise mean a container it has never seen.
            (1, 1) => format!("1 {}", t("elsewhere")),
            (1, _) => format!("1 {}", t("replica")),
            (total, 0) => format!("{total} {}", t("replicas")),
            (total, away) => format!("{total} {} · {away} {}", t("replicas"), t("elsewhere")),
        }
    }
}

pub fn where_it_runs(placements: &[crate::entities::replica::Replica]) -> Whereabouts {
    let live: Vec<&crate::entities::replica::Replica> =
        placements.iter().filter(|r| !r.evicted()).collect();

    Whereabouts {
        total: live.len(),
        away: live.iter().filter(|r| !r.is_here()).count(),
    }
}

/// What the service's own node can see, when nothing of it runs there.
///
/// A service placed entirely on other machines used to read `Absent`
/// here, which the badge says as **"Not deployed"** — and it is deployed,
/// it is running, just not on the machine being asked. Reported by Jorge
/// with the page beside it: a copy `Running` on another node, and a badge
/// over it saying the service had never been deployed.
///
/// Two answers rather than one, because the page must not invent an
/// outcome nobody reported: a copy elsewhere is only known to be up once
/// that node has said so, which it does when it collects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elsewhere {
    /// Some copy on another node has reported an address.
    Running,
    /// Placed, and nothing has come back about it yet.
    Silent,
}

/// What the copies on other nodes amount to, when none of them is here.
///
/// `None` the moment a copy runs on this node: `observed` is the answer
/// then, and this is not. One definition because three places ask —
/// the project's list, its stream, and the service's own page — and a
/// badge that disagreed with itself between a first paint and the swap
/// that followed is the failure `StateCell` exists to prevent.
pub fn elsewhere_of(placements: &[crate::entities::replica::Replica]) -> Option<Elsewhere> {
    let live = placements.iter().filter(|replica| !replica.evicted());
    if live.clone().any(|replica| replica.is_here()) {
        return None;
    }
    let mut away = live.filter(|replica| !replica.is_here()).peekable();
    away.peek()?;
    match away.any(|replica| replica.address.is_some()) {
        true => Some(Elsewhere::Running),
        false => Some(Elsewhere::Silent),
    }
}

pub fn state_cell(
    observed: &crate::deploy::Observed,
    deploying: bool,
    whereabouts: Whereabouts,
    // `None` when copies run on this node, which is the ordinary case and
    // the one `observed` already answers for.
    elsewhere: Option<Elsewhere>,
) -> StateCell {
    // Empty until something says it: the sentence needs a language, and
    // this runs in a handler, which has none around it.
    let address = String::new();
    if deploying {
        return StateCell {
            word: "Deploying".into(),
            badge: "badge badge-info",
            dot: "dot dot-info dot-pulse",
            action: "stop",
            busy: true,
            address,
            whereabouts,
        };
    }
    match observed {
        crate::deploy::Observed::Running { .. } => StateCell {
            word: "Running".into(),
            badge: "badge badge-success",
            dot: "dot dot-success",
            action: "stop",
            busy: false,
            address,
            whereabouts,
        },
        crate::deploy::Observed::Stopped { exit_code } => StateCell {
            word: format!("Exited {exit_code}"),
            badge: "badge badge-danger",
            dot: "dot dot-danger",
            action: "deploy",
            busy: false,
            address: address.clone(),
            whereabouts: whereabouts.clone(),
        },
        // Nothing here. Which is not the same as nothing anywhere, and
        // the difference is the whole of what this argument is for — the
        // action included: stopping it is what its owner can do about a
        // service running on somebody else's machine, and the instruction
        // travels now.
        crate::deploy::Observed::Absent => match elsewhere {
            Some(Elsewhere::Running) => StateCell {
                word: "Running elsewhere".into(),
                badge: "badge badge-success",
                dot: "dot dot-success",
                action: "stop",
                busy: false,
                address: address.clone(),
                whereabouts: whereabouts.clone(),
            },
            Some(Elsewhere::Silent) => StateCell {
                word: "Waiting for that node".into(),
                badge: "badge badge-info",
                dot: "dot dot-info dot-pulse",
                action: "stop",
                busy: false,
                address: address.clone(),
                whereabouts: whereabouts.clone(),
            },
            None => StateCell {
                word: "Not deployed".into(),
                badge: "badge badge-warning",
                dot: "dot dot-warning",
                action: "deploy",
                busy: false,
                address: address.clone(),
                whereabouts: whereabouts.clone(),
            },
        },
        crate::deploy::Observed::Unknown(_) => StateCell {
            word: "Unknown".into(),
            badge: "badge badge-info",
            dot: "dot dot-info",
            action: "deploy",
            busy: false,
            address: address.clone(),
            whereabouts: whereabouts.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::Observed;
    use crate::views::components::project_cells::replica_cell;

    /// The column counts copies and never names an address.
    ///
    /// It named one while there was exactly one copy, which made the
    /// column answer two different questions a row apart — an address on
    /// this row, a count on the next, under a heading that could only
    /// suit one of them. And the address it named is reachable from
    /// inside the project only, where the service's own name resolves
    /// and is the thing to use. Which copy is where is the replicas
    /// table, a row each.
    #[test]
    fn the_column_counts_copies_rather_than_naming_one() {
        let replica = |slot, here: bool, address: Option<&str>| crate::entities::replica::Replica {
            id: format!("rp-{slot}"),
            service_id: "svc-1".into(),
            node_id: (!here).then(|| "nd-elsewhere".to_string()),
            slot,
            address: address.map(str::to_string),
            overlay_port: None,
            last_error: None,
            evicted_at: None,
            reserved_host: None,
            memory_bytes: None,
            disk_bytes: None,
            cpu_millicores: None,
            following: None,
            wal_held: None,
            placed_by: None,
            placed_note: None,
        };

        // The ordinary case: one copy, here, with an address the page
        // no longer shows.
        assert_eq!(
            where_it_runs(&[replica(1, true, Some("10.42.1.5"))]).say(),
            "1 replica"
        );
        // One copy and it is not on this machine, which is worth saying
        // even alone: "running" here would otherwise mean a container
        // this node has never seen.
        assert_eq!(
            where_it_runs(&[replica(1, false, None)]).say(),
            "1 elsewhere"
        );
        // Placed nowhere at all — not "0 replicas", which reads as a
        // measurement of something.
        assert_eq!(where_it_runs(&[]).say(), "nowhere");

        assert_eq!(
            where_it_runs(&[replica(1, true, Some("10.42.1.5")), replica(2, true, None)]).say(),
            "2 replicas"
        );
        assert_eq!(
            where_it_runs(&[
                replica(1, true, Some("10.42.1.5")),
                replica(2, false, None),
                replica(3, false, None),
            ])
            .say(),
            "3 replicas · 2 elsewhere"
        );
    }

    /// The count is a sentence, and a sentence on this console is read
    /// in the account's language. It was composed in the handler out of
    /// English words and reached neither door — not `t`, which takes a
    /// literal, and not the stream's `translate`, which looks a whole
    /// value up in a table. So a Spanish page said "3 replicas · 2
    /// elsewhere", in the one column that changes as copies move.
    #[test]
    fn the_count_of_copies_is_read_in_the_accounts_language() {
        let three = Whereabouts { total: 3, away: 2 };
        let spanish = crate::views::texts::language::scoped(
            crate::views::texts::language::Language::Es,
            || three.say(),
        );
        assert_eq!(spanish, "3 réplicas · 2 en otro nodo");

        assert_eq!(
            crate::views::texts::language::scoped(
                crate::views::texts::language::Language::Es,
                || { Whereabouts { total: 1, away: 1 }.say() }
            ),
            "1 en otro nodo"
        );
        // Singular, and the plural is not "1 replicas".
        assert_eq!(
            crate::views::texts::language::scoped(
                crate::views::texts::language::Language::Es,
                || { Whereabouts { total: 1, away: 0 }.say() }
            ),
            "1 réplica"
        );
    }

    /// A badge is prose, and the page around it is in the account's
    /// language — so a Spanish console with an English badge on it is the
    /// page contradicting itself, which is what Jorge's screenshot showed.
    ///
    /// The source scan in `es.rs` cannot find these: they are picked here
    /// as values and carried to the render on a `StateCell`, not asked for
    /// with `t`. So this is the guard, and it has to name every word the
    /// function can produce — bar `Exited n`, which is containerd's word
    /// and a number.
    #[test]
    fn a_state_word_is_a_word_somebody_reads() {
        // This test is about the word; where it runs is not part of it.
        let nowhere = || Whereabouts { total: 0, away: 0 };
        let placed = [
            state_cell(&Observed::Absent, true, nowhere(), None),
            state_cell(&Observed::Absent, false, nowhere(), None),
            state_cell(
                &Observed::Absent,
                false,
                nowhere(),
                Some(Elsewhere::Running),
            ),
            state_cell(&Observed::Absent, false, nowhere(), Some(Elsewhere::Silent)),
            state_cell(
                &Observed::Running {
                    pid: 1,
                    address: None,
                },
                false,
                nowhere(),
                None,
            ),
            state_cell(&Observed::Unknown("busy".into()), false, nowhere(), None),
        ];

        for cell in &placed {
            assert!(
                crate::views::texts::lookup(&cell.word).is_some(),
                "no Spanish for the state word {:?}",
                cell.word
            );
        }

        // And the words of a *replica*, which reach a render the same
        // way and were guarded by nothing. They happened to be
        // translated; the first one added after this test was written —
        // "Not answering" — was not, and the whole suite stayed green.
        // A guard that covers one of two functions producing the same
        // kind of value is a guard that will be right until somebody
        // touches the other one.
        let replica = |address: Option<&str>| crate::entities::replica::Replica {
            id: "rp-1".into(),
            service_id: "svc-1".into(),
            node_id: Some("nd-far".into()),
            slot: 2,
            address: address.map(str::to_string),
            overlay_port: None,
            last_error: None,
            evicted_at: None,
            reserved_host: None,
            memory_bytes: None,
            disk_bytes: None,
            cpu_millicores: None,
            following: None,
            wal_held: None,
            placed_by: None,
            placed_note: None,
        };
        let failed = crate::entities::replica::Replica {
            last_error: Some("it exited 1".into()),
            ..replica(None)
        };
        let evicted = crate::entities::replica::Replica {
            evicted_at: Some(1),
            ..replica(None)
        };

        let not_following = crate::entities::replica::Replica {
            following: Some(false),
            ..replica(Some("10.42.1.5"))
        };
        let copies = [
            replica_cell(&evicted, false, false, false),
            replica_cell(&failed, false, false, false),
            replica_cell(&replica(Some("10.42.1.5")), false, false, false),
            replica_cell(&replica(Some("10.42.1.5")), false, false, true),
            replica_cell(&not_following, false, false, false),
            replica_cell(&replica(None), false, false, false),
            replica_cell(&replica(None), true, false, false),
            replica_cell(&replica(None), false, true, false),
        ];
        for cell in &copies {
            assert!(
                crate::views::texts::lookup(&cell.word).is_some(),
                "no Spanish for the replica word {:?}",
                cell.word
            );
        }
    }

    /// During a deployment containerd's answer is a half-truth — the
    /// old container is gone and the new one is not up — so "absent"
    /// there reads as a fault. The job is what knows.
    #[test]
    fn a_deployment_in_flight_outranks_what_containerd_says() {
        let nowhere = || Whereabouts { total: 0, away: 0 };
        let busy = state_cell(&Observed::Absent, true, nowhere(), None);
        assert_eq!(busy.word, "Deploying");
        // Shown, not hidden: a control that vanishes takes the column's
        // width with it. Disabled says the same thing and keeps the row
        // still — and it names where the deployment is heading.
        assert_eq!(busy.action, "stop");
        assert!(busy.busy, "and it cannot be pressed yet");

        let idle = state_cell(&Observed::Absent, false, nowhere(), None);
        assert_eq!(idle.word, "Not deployed");
        assert_eq!(idle.action, "deploy");
        assert!(!idle.busy);

        // Running offers Stop, which is the pairing the row used to get
        // wrong the moment the badge updated and the button did not.
        let running = state_cell(
            &Observed::Running {
                pid: 1,
                address: None,
            },
            false,
            Whereabouts { total: 2, away: 1 },
            None,
        );
        assert_eq!(running.action, "stop");
        // Where it runs rides with the state: a deployment ends by
        // placing copies, and a row showing the new state beside the
        // old count is half-updated.
        assert_eq!(running.whereabouts.say(), "2 replicas · 1 elsewhere");
    }
}
